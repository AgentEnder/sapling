import os
import time
import logging

import warnings
import mysql.connector

from indexapi import (
    indexapi,
    indexexception,
)

class sqlindexapi(indexapi):
    '''
    Sql backend for infinitepush index. See tables.

    CREATE TABLE IF NOT EXISTS nodestobundle(
    node CHAR(40) BINARY NOT NULL,
    bundle VARCHAR(512) BINARY NOT NULL,
    reponame CHAR(255) BINARY NOT NULL,
    PRIMARY KEY(node, reponame));

    CREATE TABLE IF NOT EXISTS bookmarkstonode(
    node CHAR(40) BINARY NOT NULL,
    bookmark VARCHAR(512) BINARY NOT NULL,
    reponame CHAR(255) BINARY NOT NULL,
    PRIMARY KEY(reponame, bookmark));

    CREATE TABLE IF NOT EXISTS bundles(
    bundle VARCHAR(512) BINARY NOT NULL,
    reponame CHAR(255) BINARY NOT NULL,
    PRIMARY KEY(bundle, reponame));
    '''

    def __init__(self, reponame, host, port,
                 database, user, password, logfile, loglevel):
        super(sqlindexapi, self).__init__()
        self.reponame = reponame
        self.sqlargs = {
            'host': host,
            'port': port,
            'database': database,
            'user': user,
            'password': password,
        }
        self.sqlconn = None
        self.sqlcursor = None
        if not logfile:
            logfile = os.devnull
        logging.basicConfig(filename=logfile)
        self.log = logging.getLogger()
        self.log.setLevel(loglevel)
        self._connected = False

    def sqlconnect(self):
        if self.sqlconn:
            raise indexexception("SQL connection already open")
        if self.sqlcursor:
            raise indexexception("SQL cursor already open without connection")
        retry = 3
        while True:
            try:
                self.sqlconn = mysql.connector.connect(
                    force_ipv6=True, **self.sqlargs)

                # Code is copy-pasted from hgsql. Bug fixes need to be
                # back-ported!
                # The default behavior is to return byte arrays, when we
                # need strings. This custom convert returns strings.
                self.sqlconn.set_converter_class(CustomConverter)
                self.sqlconn.autocommit = False
                break
            except mysql.connector.errors.Error:
                # mysql can be flakey occasionally, so do some minimal
                # retrying.
                retry -= 1
                if retry == 0:
                    raise
                time.sleep(0.2)

        waittimeout = 300
        waittimeout = self.sqlconn.converter.escape('%s' % waittimeout)

        self.sqlcursor = self.sqlconn.cursor()
        self._connected = True

    def close(self):
        """Cleans up the metadata store connection."""
        with warnings.catch_warnings():
            warnings.simplefilter("ignore")
            self.sqlcursor.close()
            self.sqlconn.close()
        self.sqlcursor = None
        self.sqlconn = None

    def addbundle(self, bundleid, nodes, commit=True):
        """Takes a bundleid and a list of nodes in that bundle and records that
        each node is contained in that bundle."""
        if not self._connected:
            self.sqlconnect()
        self.log.info("ADD BUNDLE %r %r %r" % (self.reponame, bundleid, nodes))
        self.sqlcursor.execute(
            "INSERT INTO bundles(bundle, reponame) VALUES "
            "(%s, %s)", params=(bundleid, self.reponame))
        for node in nodes:
            self.sqlcursor.execute(
                "INSERT INTO nodestobundle(node, bundle, reponame) "
                "VALUES (%s, %s, %s) ON DUPLICATE KEY UPDATE "
                "bundle=VALUES(bundle)",
                params=(node, bundleid, self.reponame))
        if commit:
            self.sqlconn.commit()

    def addbookmark(self, bookmark, node, commit=True):
        """Takes a bookmark name and hash, and records mapping in the metadata
        store."""
        if not self._connected:
            self.sqlconnect()
        self.log.info(
            "ADD BOOKMARKS %r bookmark: %r node: %r" %
            (self.reponame, bookmark, node))
        self.sqlcursor.execute(
            "INSERT INTO bookmarkstonode(bookmark, node, reponame) "
            "VALUES (%s, %s, %s) ON DUPLICATE KEY UPDATE node=VALUES(node)",
            params=(bookmark, node, self.reponame))
        if commit:
            self.sqlconn.commit()

    def addbookmarkandbundle(self, bundleid, nodes, bookmark, bookmarknode):
        if not self._connected:
            self.sqlconnect()
        self.addbundle(bundleid, nodes, commit=False)
        self.addbookmark(bookmark, bookmarknode, commit=False)
        self.sqlconn.commit()

    def listbookmarks(self):
        if not self._connected:
            self.sqlconnect()
        self.log.info("LIST BOOKMARKS")
        self.sqlcursor.execute("SELECT bookmark, node from bookmarkstonode")
        result = self.sqlcursor.fetchall()
        if not self._connected:
            self.sqlconnect()
        self.log.info("Found %d bookmarks")
        bookmarks = {}
        for row in result:
            bookmarks[row[0]] = row[1]
        return bookmarks

    def getbundle(self, node):
        """Returns the bundleid for the bundle that contains the given node."""
        if not self._connected:
            self.sqlconnect()
        self.log.info("GET BUNDLE %r %r" % (self.reponame, node))
        self.sqlcursor.execute(
            "SELECT bundle from nodestobundle "
            "WHERE node = %s AND reponame = %s", params=(node, self.reponame))
        result = self.sqlcursor.fetchall()
        if len(result) != 1 or len(result[0]) != 1:
            self.log.info("No matching node")
            return None
        bundle = result[0][0]
        self.log.info("Found bundle %r" % bundle)
        return bundle

    def getnode(self, bookmark):
        """Returns the node for the given bookmark. None if it doesn't exist."""
        if not self._connected:
            self.sqlconnect()
        self.log.info(
            "GET NODE reponame: %r bookmark: %r" % (self.reponame, bookmark))
        self.sqlcursor.execute(
            "SELECT node from bookmarkstonode WHERE "
            "bookmark = %s AND reponame = %s", params=(bookmark, self.reponame))
        result = self.sqlcursor.fetchall()
        if len(result) != 1 or len(result[0]) != 1:
            self.log.info("No matching bookmark")
            return None
        node = result[0][0]
        self.log.info("Found node %r" % node)
        return node

    def getbookmarks(self, query):
        if not self._connected:
            self.sqlconnect()
        self.log.info(
            "QUERY BOOKMARKS reponame: %r query: %r" % (self.reponame, query))
        query = query.replace('_', '\\_')
        query = query.replace('%', '\\%')
        if query.endswith('*'):
            query = query[:-1] + '%'
        self.sqlcursor.execute(
            "SELECT bookmark, node from bookmarkstonode WHERE "
            "reponame = %s AND bookmark LIKE %s",
            params=(self.reponame, query))
        result = self.sqlcursor.fetchall()
        bookmarks = {}
        for row in result:
            if len(row) != 2:
                self.log.info("Bad row returned: %s" % row)
                continue
            bookmarks[row[0]] = row[1]
        return bookmarks

class CustomConverter(mysql.connector.conversion.MySQLConverter):
    """Ensure that all values being returned are returned as python string
    (versus the default byte arrays)."""
    def _STRING_to_python(self, value, dsc=None):
        return str(value)

    def _VAR_STRING_to_python(self, value, dsc=None):
        return str(value)

    def _BLOB_to_python(self, value, dsc=None):
        return str(value)
