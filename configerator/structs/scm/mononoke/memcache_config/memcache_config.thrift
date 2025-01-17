// @generated SignedSource<<02bc4018b3abef9a279095898ab3aad9>>
// DO NOT EDIT THIS FILE MANUALLY!
// This file is a mechanical copy of the version in the configerator repo. To
// modify it, edit the copy in the configerator repo instead and copy it over by
// running the following in your fbcode directory:
//
// configerator-thrift-updater scm/mononoke/memcache_config/memcache_config.thrift

namespace py configerator.memcache_config

struct MemcacheConfig {
	// Should memcache be used? False means that writes silently discard, reads always return "not present"
	1: bool enable;
	// Per-cache sitevers
	2: i32 apiserver_sitever;
	3: i32 blobstore_sitever;
	4: i32 filenodes_sitever;
	5: i32 changesets_sitever;
	6: i32 phases_sitever;
	7: i32 bonsai_hg_mapping_sitever;
}
