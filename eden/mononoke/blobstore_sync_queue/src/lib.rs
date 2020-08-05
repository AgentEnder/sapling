/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

#![deny(warnings)]

use anyhow::{format_err, Error};
use async_trait::async_trait;
use auto_impl::auto_impl;
use cloned::cloned;
use context::CoreContext;
use futures::{
    channel::{mpsc, oneshot},
    compat::Future01CompatExt,
    future::{self, BoxFuture, FutureExt, Shared, TryFutureExt},
    stream::StreamExt,
};
use metaconfig_types::{BlobstoreId, MultiplexId};
use mononoke_types::{DateTime, Timestamp};
use sql::mysql_async::{
    prelude::{ConvIr, FromValue},
    FromValueError, Value,
};
use sql::{queries, Connection};
pub use sql_construct::SqlConstruct;
pub use sql_ext::SqlConnections;
use stats::prelude::*;
use std::iter::IntoIterator;
use std::sync::Arc;
use uuid::Uuid;

define_stats! {
    prefix = "mononoke.blobstore_sync_queue";
    adds: timeseries(Rate, Sum),
    iters: timeseries(Rate, Sum),
    dels: timeseries(Rate, Sum),
}

// Identifier for given blobstore operation to faciliate correlating same operation
// across multiple blobstores.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct OperationKey(pub Uuid);
impl OperationKey {
    pub fn gen() -> OperationKey {
        OperationKey(Uuid::new_v4())
    }

    pub fn is_null(&self) -> bool {
        self == &OperationKey(Uuid::nil())
    }
}

impl From<OperationKey> for Value {
    fn from(id: OperationKey) -> Self {
        let OperationKey(uuid) = id;
        Value::Bytes(uuid.as_bytes().to_vec())
    }
}

impl ConvIr<OperationKey> for OperationKey {
    fn new(v: Value) -> Result<Self, FromValueError> {
        match v {
            Value::Bytes(bytes) => Ok(OperationKey(
                Uuid::from_slice(&bytes[..])
                    .map_err(move |_| FromValueError(Value::Bytes(bytes)))?,
            )),
            v => Err(FromValueError(v)),
        }
    }

    fn commit(self) -> Self {
        self
    }

    fn rollback(self) -> Value {
        self.into()
    }
}

impl FromValue for OperationKey {
    type Intermediate = OperationKey;
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct BlobstoreSyncQueueEntry {
    pub blobstore_key: String,
    pub blobstore_id: BlobstoreId,
    pub multiplex_id: MultiplexId,
    pub timestamp: DateTime,
    pub id: Option<u64>,
    pub operation_key: OperationKey,
}

impl BlobstoreSyncQueueEntry {
    pub fn new(
        blobstore_key: String,
        blobstore_id: BlobstoreId,
        multiplex_id: MultiplexId,
        timestamp: DateTime,
        operation_key: OperationKey,
    ) -> Self {
        Self {
            blobstore_key,
            blobstore_id,
            multiplex_id,
            timestamp,
            operation_key,
            id: None,
        }
    }
}

#[async_trait]
#[auto_impl(Arc, Box)]
pub trait BlobstoreSyncQueue: Send + Sync {
    async fn add<'a>(
        &'a self,
        ctx: &'a CoreContext,
        entry: BlobstoreSyncQueueEntry,
    ) -> Result<(), Error> {
        self.add_many(ctx, vec![entry]).await
    }

    async fn add_many<'a>(
        &'a self,
        ctx: &'a CoreContext,
        entries: Vec<BlobstoreSyncQueueEntry>,
    ) -> Result<(), Error>;

    /// Returns list of entries that consist of two groups of entries:
    /// 1. Group with at most `limit` entries that are older than `older_than` and
    ///    optionally sql like `key_like`
    /// 2. Group of entries whose `blobstore_key` can be found in group (1)
    ///
    /// As a result the caller gets a reasonably limited slice of BlobstoreSyncQueue entries that
    /// are all related, so that the caller doesn't need to fetch more data from BlobstoreSyncQueue
    /// to process the sync queue.
    async fn iter<'a>(
        &'a self,
        ctx: &'a CoreContext,
        key_like: Option<&'a String>,
        multiplex_id: MultiplexId,
        older_than: DateTime,
        limit: usize,
    ) -> Result<Vec<BlobstoreSyncQueueEntry>, Error>;

    async fn del<'a>(
        &'a self,
        ctx: &'a CoreContext,
        entries: &'a [BlobstoreSyncQueueEntry],
    ) -> Result<(), Error>;

    async fn get<'a>(
        &'a self,
        ctx: &'a CoreContext,
        key: &'a String,
    ) -> Result<Vec<BlobstoreSyncQueueEntry>, Error>;
}

#[derive(Clone)]
pub struct SqlBlobstoreSyncQueue {
    write_connection: Arc<Connection>,
    read_connection: Connection,
    read_master_connection: Connection,
    write_sender:
        Arc<mpsc::UnboundedSender<(oneshot::Sender<Result<(), Error>>, BlobstoreSyncQueueEntry)>>,
    ensure_worker_scheduled: Shared<BoxFuture<'static, ()>>,
}

queries! {
    write InsertEntry(values: (
        blobstore_key: String,
        blobstore_id: BlobstoreId,
        multiplex_id: MultiplexId,
        timestamp: Timestamp,
        operation_key: OperationKey,
    )) {
        none,
        "INSERT INTO blobstore_sync_queue (blobstore_key, blobstore_id, multiplex_id, add_timestamp, operation_key)
         VALUES {values}"
    }

    write DeleteEntries(>list ids: u64) {
        none,
        "DELETE FROM blobstore_sync_queue WHERE id in {ids}"
    }

    read GetRangeOfEntries(multiplex_id: MultiplexId, older_than: Timestamp, limit: usize) -> (
        String,
        BlobstoreId,
        MultiplexId,
        Timestamp,
        OperationKey,
        u64,
    ) {
        "SELECT blobstore_key, blobstore_id, multiplex_id, add_timestamp, blobstore_sync_queue.operation_key, id
         FROM blobstore_sync_queue
         JOIN (
               SELECT DISTINCT operation_key
               FROM blobstore_sync_queue
               WHERE add_timestamp <= {older_than} AND multiplex_id = {multiplex_id}
               LIMIT {limit}
         ) b
         ON blobstore_sync_queue.operation_key = b.operation_key AND multiplex_id = {multiplex_id}
         "
    }

    read GetRangeOfEntriesLike(blobstore_key_like: String, multiplex_id: MultiplexId, older_than: Timestamp, limit: usize) -> (
        String,
        BlobstoreId,
        MultiplexId,
        Timestamp,
        OperationKey,
        u64,
    ) {
        "SELECT blobstore_key, blobstore_id, multiplex_id, add_timestamp, blobstore_sync_queue.operation_key, id
         FROM blobstore_sync_queue
         JOIN (
               SELECT DISTINCT operation_key
               FROM blobstore_sync_queue
               WHERE blobstore_key LIKE {blobstore_key_like} AND add_timestamp <= {older_than} AND multiplex_id = {multiplex_id}
               LIMIT {limit}
         ) b
         ON blobstore_sync_queue.operation_key = b.operation_key AND multiplex_id = {multiplex_id}
         "
    }

    read GetByKey(key: String) -> (
        String,
        BlobstoreId,
        MultiplexId,
        Timestamp,
        OperationKey,
        u64,
    ) {
        "SELECT blobstore_key, blobstore_id, multiplex_id, add_timestamp, operation_key, id
         FROM blobstore_sync_queue
         WHERE blobstore_key = {key}"
    }
}

impl SqlConstruct for SqlBlobstoreSyncQueue {
    const LABEL: &'static str = "blobstore_sync_queue";

    const CREATION_QUERY: &'static str = include_str!("../schemas/sqlite-blobstore-sync-queue.sql");

    fn from_sql_connections(connections: SqlConnections) -> Self {
        let write_connection = Arc::new(connections.write_connection);
        type ChannelType = (oneshot::Sender<Result<(), Error>>, BlobstoreSyncQueueEntry);
        let (sender, receiver): (mpsc::UnboundedSender<ChannelType>, _) = mpsc::unbounded();

        let ensure_worker_scheduled = {
            cloned!(write_connection);
            async move {
                let batch_writes = receiver.ready_chunks(WRITE_BUFFER_SIZE).for_each({
                    move |batch| {
                        cloned!(write_connection);
                        async move {
                            let (senders, entries): (Vec<_>, Vec<_>) = batch.into_iter().unzip();

                            match insert_entries(write_connection.as_ref(), entries).await {
                                Ok(()) => {
                                    for sender in senders {
                                        // Ignoring the error, because receiver might have gone
                                        let _ = sender.send(Ok(()));
                                    }
                                }
                                Err(err) => {
                                    let s = format!("failed to insert {}", err);
                                    for sender in senders {
                                        // Ignoring the error, because receiver might have gone
                                        let _ = sender.send(Err(Error::msg(s.clone())));
                                    }
                                }
                            }
                        }
                    }
                });

                tokio::spawn(batch_writes);
            }
        }
        .boxed()
        .shared();

        Self {
            write_connection,
            read_connection: connections.read_connection,
            read_master_connection: connections.read_master_connection,
            write_sender: Arc::new(sender),
            ensure_worker_scheduled,
        }
    }
}

const WRITE_BUFFER_SIZE: usize = 5000;

async fn insert_entries(
    write_connection: &Connection,
    entries: Vec<BlobstoreSyncQueueEntry>,
) -> Result<(), Error> {
    let entries: Vec<_> = entries
        .into_iter()
        .map(|entry| {
            let BlobstoreSyncQueueEntry {
                blobstore_key,
                blobstore_id,
                timestamp,
                multiplex_id,
                operation_key,
                ..
            } = entry;
            let t: Timestamp = timestamp.into();
            (blobstore_key, blobstore_id, multiplex_id, t, operation_key)
        })
        .collect();

    let entries_ref: Vec<_> = entries
        .iter()
        .map(|(b, c, d, e, f)| (b, c, d, e, f)) // &(a, b, ...) into (&a, &b, ...)
        .collect();

    InsertEntry::query(write_connection, entries_ref.as_ref())
        .compat()
        .await?;
    Ok(())
}

#[async_trait]
impl BlobstoreSyncQueue for SqlBlobstoreSyncQueue {
    async fn add_many<'a>(
        &'a self,
        _ctx: &'a CoreContext,
        entries: Vec<BlobstoreSyncQueueEntry>,
    ) -> Result<(), Error> {
        self.ensure_worker_scheduled.clone().await;
        let (senders_entries, receivers): (Vec<_>, Vec<_>) = entries
            .into_iter()
            .map(|entry| {
                let (sender, receiver) = oneshot::channel();
                ((sender, entry), receiver)
            })
            .unzip();

        STATS::adds.add_value(senders_entries.len() as i64);
        senders_entries
            .into_iter()
            .map(|(send, entry)| self.write_sender.unbounded_send((send, entry)))
            .collect::<Result<_, _>>()?;
        let results = future::try_join_all(receivers)
            .map_err(|errs| format_err!("failed to receive result {:?}", errs))
            .await?;
        let errs: Vec<_> = results.into_iter().filter_map(|r| r.err()).collect();
        if errs.len() > 0 {
            Err(format_err!("failed to receive result {:?}", errs))
        } else {
            Ok(())
        }
    }

    async fn iter<'a>(
        &'a self,
        _ctx: &'a CoreContext,
        key_like: Option<&'a String>,
        multiplex_id: MultiplexId,
        older_than: DateTime,
        limit: usize,
    ) -> Result<Vec<BlobstoreSyncQueueEntry>, Error> {
        STATS::iters.add_value(1);
        let rows = match key_like {
            Some(sql_like) => {
                GetRangeOfEntriesLike::query(
                    &self.read_master_connection,
                    sql_like,
                    &multiplex_id,
                    &older_than.into(),
                    &limit,
                )
                .compat()
                .await
            }
            None => {
                GetRangeOfEntries::query(
                    &self.read_master_connection,
                    &multiplex_id,
                    &older_than.into(),
                    &limit,
                )
                .compat()
                .await
            }
        }?;

        Ok(rows
            .into_iter()
            .map(
                |(blobstore_key, blobstore_id, multiplex_id, timestamp, operation_key, id)| {
                    BlobstoreSyncQueueEntry {
                        blobstore_key,
                        blobstore_id,
                        multiplex_id,
                        timestamp: timestamp.into(),
                        operation_key,
                        id: Some(id),
                    }
                },
            )
            .collect())
    }

    async fn del<'a>(
        &'a self,
        _ctx: &'a CoreContext,
        entries: &'a [BlobstoreSyncQueueEntry],
    ) -> Result<(), Error> {
        let ids: Vec<u64> = entries
            .into_iter()
            .map(|entry| {
                entry.id.ok_or_else(|| {
                    format_err!("BlobstoreSyncQueueEntry must contain `id` to be able to delete it")
                })
            })
            .collect::<Result<_, _>>()?;

        for chunk in ids.chunks(10_000) {
            let deletion_result = DeleteEntries::query(&self.write_connection, chunk)
                .compat()
                .await?;
            STATS::dels.add_value(deletion_result.affected_rows() as i64);
        }
        Ok(())
    }

    async fn get<'a>(
        &'a self,
        _ctx: &'a CoreContext,
        key: &'a String,
    ) -> Result<Vec<BlobstoreSyncQueueEntry>, Error> {
        let rows = GetByKey::query(&self.read_master_connection, key)
            .compat()
            .await?;
        Ok(rows
            .into_iter()
            .map(
                |(blobstore_key, blobstore_id, multiplex_id, timestamp, operation_key, id)| {
                    BlobstoreSyncQueueEntry {
                        blobstore_key,
                        blobstore_id,
                        multiplex_id,
                        timestamp: timestamp.into(),
                        operation_key,
                        id: Some(id),
                    }
                },
            )
            .collect())
    }
}
