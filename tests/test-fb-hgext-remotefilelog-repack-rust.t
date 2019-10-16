  $ setconfig extensions.treemanifest=!

  $ . "$TESTDIR/library.sh"

  $ hginit master
  $ cd master
  $ cat >> .hg/hgrc <<EOF
  > [remotefilelog]
  > server=True
  > serverexpiration=-1
  > EOF
  $ echo x > x
  $ hg commit -qAm x
  $ echo x >> x
  $ hg commit -qAm x2
  $ cd ..

  $ hgcloneshallow ssh://user@dummy/master shallow -q
  1 files fetched over 1 fetches - (1 misses, 0.00% hit ratio) over *s (glob)

# Set the prefetchdays config to zero so that all commits are prefetched
# no matter what their creation date is.
  $ cd shallow
  $ cat >> .hg/hgrc <<EOF
  > [remotefilelog]
  > prefetchdays=0
  > EOF
  $ cd ..

  $ cd master
  $ echo x3 > x
  $ hg commit -qAm x3
  $ echo x4 > x
  $ hg commit -qAm x4
  $ cd ../shallow
  $ hg pull -q
  $ hg up -q tip
  1 files fetched over 1 fetches - (1 misses, 0.00% hit ratio) over * (glob)

# Pack a mix of packfiles and loosefiles into one packfile
  $ hg prefetch -r 0
  1 files fetched over 1 fetches - (1 misses, 0.00% hit ratio) over * (glob)
  $ hg prefetch -r 2
  1 files fetched over 1 fetches - (1 misses, 0.00% hit ratio) over * (glob)
  $ find $CACHEDIR | sort
  $TESTTMP/hgcache
  $TESTTMP/hgcache/master
  $TESTTMP/hgcache/master/packs
  $TESTTMP/hgcache/master/packs/1e6f0f575de6319f747ef83966a08775803fcecc.dataidx
  $TESTTMP/hgcache/master/packs/1e6f0f575de6319f747ef83966a08775803fcecc.datapack
  $TESTTMP/hgcache/master/packs/276d308429d0303762befa376788300f0310f90e.histidx
  $TESTTMP/hgcache/master/packs/276d308429d0303762befa376788300f0310f90e.histpack
  $TESTTMP/hgcache/master/packs/2d66e09c3bf8a000428af1630d978127182e496e.dataidx
  $TESTTMP/hgcache/master/packs/2d66e09c3bf8a000428af1630d978127182e496e.datapack
  $TESTTMP/hgcache/master/packs/3266aa7480df06153adccad2f1abb6d11f42de0e.dataidx
  $TESTTMP/hgcache/master/packs/3266aa7480df06153adccad2f1abb6d11f42de0e.datapack
  $TESTTMP/hgcache/master/packs/3b65e3071e408ff050835eba9d2662d0c5ea51db.histidx
  $TESTTMP/hgcache/master/packs/3b65e3071e408ff050835eba9d2662d0c5ea51db.histpack
  $TESTTMP/hgcache/master/packs/887690f1138ae5b99c50d754ed02262874bf8ecb.dataidx
  $TESTTMP/hgcache/master/packs/887690f1138ae5b99c50d754ed02262874bf8ecb.datapack
  $TESTTMP/hgcache/master/packs/acb190832c13f0a23d7901bc1847ef7f6046a26e.histidx
  $TESTTMP/hgcache/master/packs/acb190832c13f0a23d7901bc1847ef7f6046a26e.histpack
  $TESTTMP/hgcache/master/packs/c3399b56e035f73c3295276ed098235a08a0ed8c.histidx
  $TESTTMP/hgcache/master/packs/c3399b56e035f73c3295276ed098235a08a0ed8c.histpack

  $ hg repack
  $ ls_l $TESTTMP/hgcache/master/packs/ | grep datapack
  -r--r--r--     253 073bc5bae3cee0940d7f1983cab3fe6754ed1407.datapack
  $ ls_l $TESTTMP/hgcache/master/packs/ | grep histpack
  -r--r--r--     336 3b65e3071e408ff050835eba9d2662d0c5ea51db.histpack

  $ hg repack

# Repacking one datapack/historypack should result in the same datapack/historypack
  $ find $CACHEDIR | sort
  $TESTTMP/hgcache
  $TESTTMP/hgcache/master
  $TESTTMP/hgcache/master/packs
  $TESTTMP/hgcache/master/packs/073bc5bae3cee0940d7f1983cab3fe6754ed1407.dataidx
  $TESTTMP/hgcache/master/packs/073bc5bae3cee0940d7f1983cab3fe6754ed1407.datapack
  $TESTTMP/hgcache/master/packs/3b65e3071e408ff050835eba9d2662d0c5ea51db.histidx
  $TESTTMP/hgcache/master/packs/3b65e3071e408ff050835eba9d2662d0c5ea51db.histpack
  $TESTTMP/hgcache/master/packs/repacklock

  $ hg cat -r . x
  x4
  $ hg cat -r '.^' x
  x3

# Corrupt a packfile to verify that repacking continues and the corrupted file is left around.
  $ cd ../master
  $ echo x5 > x
  $ hg commit -qAm x5
  $ cd ../shallow
  $ hg pull -q
  $ hg up -q tip
  1 files fetched over 1 fetches - (1 misses, 0.00% hit ratio) over * (glob)
  $ hg repack
  $ find $CACHEDIR | sort
  $TESTTMP/hgcache
  $TESTTMP/hgcache/master
  $TESTTMP/hgcache/master/packs
  $TESTTMP/hgcache/master/packs/aa3c302eff0b511bab7f0180d344971f917472c1.dataidx
  $TESTTMP/hgcache/master/packs/aa3c302eff0b511bab7f0180d344971f917472c1.datapack
  $TESTTMP/hgcache/master/packs/ed01afd8e8527fd7e3473478b25f5b665b0ddfca.histidx
  $TESTTMP/hgcache/master/packs/ed01afd8e8527fd7e3473478b25f5b665b0ddfca.histpack
  $TESTTMP/hgcache/master/packs/repacklock

  $ chmod +w $TESTTMP/hgcache/master/packs/aa3c302eff0b511bab7f0180d344971f917472c1.datapack
  $ python $TESTDIR/truncate.py --size 200 $TESTTMP/hgcache/master/packs/aa3c302eff0b511bab7f0180d344971f917472c1.datapack
  $ hg repack
  $ find $CACHEDIR | sort
  $TESTTMP/hgcache
  $TESTTMP/hgcache/master
  $TESTTMP/hgcache/master/packs
  $TESTTMP/hgcache/master/packs/aa3c302eff0b511bab7f0180d344971f917472c1.dataidx
  $TESTTMP/hgcache/master/packs/aa3c302eff0b511bab7f0180d344971f917472c1.datapack
  $TESTTMP/hgcache/master/packs/ed01afd8e8527fd7e3473478b25f5b665b0ddfca.histidx
  $TESTTMP/hgcache/master/packs/ed01afd8e8527fd7e3473478b25f5b665b0ddfca.histpack
  $TESTTMP/hgcache/master/packs/repacklock

# Verify that a non-existing directory does not fail repack
  $ cat >> .hg/hgrc <<EOF
  > [extensions]
  > treemanifest=
  > fastmanifest=
  > EOF

  $ hg repack --incremental
