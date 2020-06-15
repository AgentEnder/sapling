/*
 * Copyright (c) Facebook, Inc. and its affiliates.
 *
 * This software may be used and distributed according to the terms of the
 * GNU General Public License version 2.
 */

use anyhow::Error;
use blobstore::Loadable;
use context::CoreContext;
use fbinit::FacebookInit;
use futures::compat::Future01CompatExt;
use maplit::hashset;
use mononoke_types::RepositoryId;
use mononoke_types_mocks::hash::*;
use pushrebase::{do_pushrebase_bonsai, OntoBookmarkParams};
use sql::rusqlite::Connection as SqliteConnection;
use tests_utils::{bookmark, CreateCommitContext};

use crate::GitMappingPushrebaseHook;

#[fbinit::test]
fn pushrebase_populates_git_mapping(fb: FacebookInit) -> Result<(), Error> {
    let mut runtime = tokio_compat::runtime::Runtime::new()?;
    runtime.block_on_std(pushrebase_populates_git_mapping_impl(fb))
}

async fn pushrebase_populates_git_mapping_impl(fb: FacebookInit) -> Result<(), Error> {
    let ctx = CoreContext::test_mock(fb);
    let repo_id = RepositoryId::new(1);
    let (repo, _con) = blobrepo_factory::new_memblob_with_sqlite_connection_with_id(
        SqliteConnection::open_in_memory()?,
        repo_id.clone(),
    )?;
    let mapping = repo.bonsai_git_mapping().clone();

    let root = CreateCommitContext::new_root(&ctx, &repo).commit().await?;

    let cs1 = CreateCommitContext::new(&ctx, &repo, vec![root])
        .commit()
        .await?;

    let cs2 = CreateCommitContext::new(&ctx, &repo, vec![root])
        .add_extra("hg-git-rename-source".to_owned(), b"git".to_vec())
        .add_extra(
            "convert_revision".to_owned(),
            TWOS_GIT_SHA1.to_hex().as_bytes().to_owned(),
        )
        .commit()
        .await?
        .load(ctx.clone(), repo.blobstore())
        .compat()
        .await?;

    let book = bookmark(&ctx, &repo, "master").set_to(cs1).await?;

    let hooks = [GitMappingPushrebaseHook::new(repo.get_repoid())];
    let onto = OntoBookmarkParams::new(book);

    let rebased = do_pushrebase_bonsai(
        &ctx,
        &repo,
        &Default::default(),
        &onto,
        &hashset![cs2.clone()],
        &None,
        &hooks,
    )
    .await?
    .rebased_changesets;

    let cs2_rebased = rebased
        .iter()
        .find(|e| e.id_old == cs2.get_changeset_id())
        .ok_or(Error::msg("missing cs2"))?
        .id_new
        .load(ctx.clone(), repo.blobstore())
        .compat()
        .await?;

    let cs3 = CreateCommitContext::new(&ctx, &repo, vec![root])
        .add_extra("hg-git-rename-source".to_owned(), b"git".to_vec())
        .add_extra(
            "convert_revision".to_owned(),
            THREES_GIT_SHA1.to_hex().as_bytes().to_owned(),
        )
        .commit()
        .await?
        .load(ctx.clone(), repo.blobstore())
        .compat()
        .await?;

    let cs4 = CreateCommitContext::new(&ctx, &repo, vec![cs3.get_changeset_id()])
        .add_extra("hg-git-rename-source".to_owned(), b"git".to_vec())
        .add_extra(
            "convert_revision".to_owned(),
            FOURS_GIT_SHA1.to_hex().as_bytes().to_owned(),
        )
        .commit()
        .await?
        .load(ctx.clone(), repo.blobstore())
        .compat()
        .await?;

    let rebased = do_pushrebase_bonsai(
        &ctx,
        &repo,
        &Default::default(),
        &onto,
        &hashset![cs3.clone(), cs4.clone()],
        &None,
        &hooks,
    )
    .await?
    .rebased_changesets;

    let cs3_rebased = rebased
        .iter()
        .find(|e| e.id_old == cs3.get_changeset_id())
        .ok_or(Error::msg("missing cs3"))?
        .id_new
        .load(ctx.clone(), repo.blobstore())
        .compat()
        .await?;

    let cs4_rebased = rebased
        .iter()
        .find(|e| e.id_old == cs4.get_changeset_id())
        .ok_or(Error::msg("missing cs4"))?
        .id_new
        .load(ctx.clone(), repo.blobstore())
        .compat()
        .await?;

    assert_eq!(
        Some(TWOS_GIT_SHA1),
        mapping
            .get_git_sha1_from_bonsai(&ctx, cs2_rebased.get_changeset_id())
            .await?,
    );
    assert_eq!(
        Some(THREES_GIT_SHA1),
        mapping
            .get_git_sha1_from_bonsai(&ctx, cs3_rebased.get_changeset_id())
            .await?,
    );
    assert_eq!(
        Some(FOURS_GIT_SHA1),
        mapping
            .get_git_sha1_from_bonsai(&ctx, cs4_rebased.get_changeset_id())
            .await?,
    );

    Ok(())
}
