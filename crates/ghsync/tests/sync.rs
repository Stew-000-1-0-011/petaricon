use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use ghsync::fake::{Fake, REPO};
use ghsync::*;

const BIN: &[u8] = &[0, 159, 146, 150, 255, 0, 13, 10];

fn seeded() -> Fake {
    let f = Fake::new();
    f.commit(
        "main",
        &[
            ("README.md", Some(b"outside\n")),
            ("ui/main.slint", Some(b"export component A {}\n")),
            ("ui/sub/deep.slint", Some(b"deep\n")),
            ("ui/img/pic.bin", Some(BIN)),
            ("ui/.settings", Some(b"remote-managed")),
            ("ui/old.bak", Some(b"bak")),
        ],
    );
    f
}

fn cloned(f: &Fake) -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    clone_into(f, REPO, "main", "ui", d.path()).unwrap();
    d
}

fn read(d: &Path, rel: &str) -> Vec<u8> {
    fs::read(d.join(rel)).unwrap()
}

fn write(d: &Path, rel: &str, c: &[u8]) {
    let p = d.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(p, c).unwrap();
}

/// Every file (including `.sync`) as a snapshot.
fn snapshot(d: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in fs::read_dir(dir).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                out.insert(
                    p.strip_prefix(base).unwrap().to_string_lossy().into(),
                    fs::read(&p).unwrap(),
                );
            }
        }
    }
    let mut m = BTreeMap::new();
    walk(d, d, &mut m);
    m
}

// ---- tests -------------------------------------------------------------------------

#[test]
fn known_blob_shas() {
    assert_eq!(
        git_blob_sha(b""),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
    assert_eq!(
        git_blob_sha(b"hello\n"),
        "ce013625030ba8dba906f756967f9e9ca394464a"
    );
}

#[test]
fn clone_places_files_byte_for_byte() {
    let f = seeded();
    let d = tempfile::tempdir().unwrap();
    let st = clone_into(&f, REPO, "main", "/ui/", d.path()).unwrap();
    assert_eq!(st.folder, "ui");
    assert_eq!(st.base_commit, f.head("main"));
    assert_eq!(read(d.path(), "main.slint"), b"export component A {}\n");
    assert_eq!(read(d.path(), "sub/deep.slint"), b"deep\n");
    assert_eq!(read(d.path(), "img/pic.bin"), BIN);
    assert!(!d.path().join("README.md").exists());
    // managed names from the remote are not imported
    assert!(!d.path().join(".settings").exists());
    assert!(!d.path().join("old.bak").exists());
    assert_eq!(SyncState::load(d.path()).unwrap().unwrap(), st);
    assert_eq!(st.files.len(), 3);
    assert!(local_changes(d.path()).unwrap().is_empty());
}

#[test]
fn clone_errors() {
    let f = seeded();
    let d = tempfile::tempdir().unwrap();
    assert!(matches!(
        clone_into(&f, REPO, "main", "nothing", d.path()),
        Err(SyncError::EmptyFolder)
    ));
    assert!(matches!(
        clone_into(&f, REPO, "nope", "ui", d.path()),
        Err(SyncError::Api(ApiError::NotFound))
    ));
    clone_into(&f, REPO, "main", "ui", d.path()).unwrap();
    assert!(matches!(
        clone_into(&f, REPO, "main", "ui", d.path()),
        Err(SyncError::AlreadyLinked)
    ));
    f.0.lock().unwrap().truncated = true;
    let d2 = tempfile::tempdir().unwrap();
    assert!(matches!(
        clone_into(&f, REPO, "main", "", d2.path()),
        Err(SyncError::Api(ApiError::Truncated))
    ));
    assert!(snapshot(d2.path()).is_empty());
    // the folder's subtree is listed on its own, so a huge repo root does not matter
    let d3 = tempfile::tempdir().unwrap();
    clone_into(&f, REPO, "main", "ui/sub", d3.path()).unwrap();
    assert_eq!(read(d3.path(), "deep.slint"), b"deep\n");
    // a non-empty directory is refused
    let d4 = tempfile::tempdir().unwrap();
    write(d4.path(), "mine.txt", b"x");
    assert!(matches!(
        clone_into(&f, REPO, "main", "ui", d4.path()),
        Err(SyncError::NotEmptyDir)
    ));
    assert_eq!(snapshot(d4.path()).len(), 1);
}

#[test]
fn pull_takes_remote_only_changes() {
    let f = seeded();
    let d = cloned(&f);
    let c = f.commit(
        "main",
        &[
            ("ui/main.slint", Some(b"new\n")),
            ("ui/added.slint", Some(b"add\n")),
            ("README.md", Some(b"z")),
        ],
    );
    let r = pull(&f, d.path()).unwrap();
    let mut u = r.updated.clone();
    u.sort();
    assert_eq!(u, vec!["added.slint", "main.slint"]);
    assert_eq!(read(d.path(), "main.slint"), b"new\n");
    assert_eq!(read(d.path(), "added.slint"), b"add\n");
    assert!(r.conflicts.is_empty() && r.deleted.is_empty());
    let st = SyncState::load(d.path()).unwrap().unwrap();
    assert_eq!(st.base_commit, c);
    assert!(local_changes(d.path()).unwrap().is_empty());
}

#[test]
fn pull_keeps_local_only_changes() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    f.commit("main", &[("ui/sub/deep.slint", Some(b"deeper\n"))]);
    let r = pull(&f, d.path()).unwrap();
    assert_eq!(r.updated, vec!["sub/deep.slint"]);
    assert_eq!(r.kept_local, vec!["main.slint"]);
    assert_eq!(read(d.path(), "main.slint"), b"mine\n");
    let ch = local_changes(d.path()).unwrap();
    assert_eq!(
        ch,
        vec![Change {
            path: "main.slint".into(),
            kind: ChangeKind::Modified
        }]
    );
    // pulling again with nothing new keeps reporting it
    let r2 = pull(&f, d.path()).unwrap();
    assert_eq!(r2.kept_local, vec!["main.slint"]);
}

#[test]
fn pull_conflict_is_not_overwritten_and_take_remote_resolves() {
    let f = seeded();
    let d = cloned(&f);
    let old_base = SyncState::load(d.path()).unwrap().unwrap().base_commit;
    write(d.path(), "main.slint", b"mine\n");
    let c = f.commit(
        "main",
        &[
            ("ui/main.slint", Some(b"theirs\n")),
            ("ui/sub/deep.slint", Some(b"d2\n")),
        ],
    );
    let r = pull(&f, d.path()).unwrap();
    assert_eq!(r.conflicts, vec!["main.slint"]);
    assert_eq!(r.updated, vec!["sub/deep.slint"]);
    assert_eq!(read(d.path(), "main.slint"), b"mine\n");
    let st = SyncState::load(d.path()).unwrap().unwrap();
    assert_eq!(
        st.base_commit, old_base,
        "base must not advance while conflicts remain"
    );
    assert!(matches!(
        push(&f, d.path(), "m"),
        Err(SyncError::ConflictsPending)
    ));

    take_remote(&f, d.path(), "main.slint").unwrap();
    assert_eq!(read(d.path(), "main.slint"), b"theirs\n");
    assert_eq!(read(d.path(), "main.slint.local"), b"mine\n");
    let st = SyncState::load(d.path()).unwrap().unwrap();
    assert_eq!(st.base_commit, c);
    assert!(st.conflicts.is_empty() && st.pending_head.is_none());
    // .local is a management file: not a change
    assert!(local_changes(d.path()).unwrap().is_empty());
}

#[test]
fn take_remote_uses_the_pulled_commit_not_a_newer_one() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    f.commit("main", &[("ui/main.slint", Some(b"theirs\n"))]);
    pull(&f, d.path()).unwrap();
    f.commit("main", &[("ui/main.slint", Some(b"even newer\n"))]);
    take_remote(&f, d.path(), "main.slint").unwrap();
    assert_eq!(read(d.path(), "main.slint"), b"theirs\n");
}

#[test]
fn take_remote_rejects_bad_paths() {
    let f = seeded();
    let d = cloned(&f);
    assert!(take_remote(&f, d.path(), "../x").is_err());
    assert!(take_remote(&f, d.path(), ".sync").is_err());
    assert!(matches!(
        take_remote(&f, d.path(), "main.slint"),
        Err(SyncError::NotAConflict(_))
    ));
    assert!(matches!(
        keep_local(&f, d.path(), "main.slint"),
        Err(SyncError::NotAConflict(_))
    ));
}

#[test]
fn remote_deletions() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "sub/deep.slint", b"edited\n");
    f.commit(
        "main",
        &[("ui/main.slint", None), ("ui/sub/deep.slint", None)],
    );
    let r = pull(&f, d.path()).unwrap();
    assert_eq!(r.deleted, vec!["main.slint"]);
    assert_eq!(r.conflicts, vec!["sub/deep.slint"]);
    assert!(!d.path().join("main.slint").exists());
    assert_eq!(read(d.path(), "sub/deep.slint"), b"edited\n");
    take_remote(&f, d.path(), "sub/deep.slint").unwrap();
    assert!(!d.path().join("sub/deep.slint").exists());
    assert_eq!(read(d.path(), "sub/deep.slint.local"), b"edited\n");
    let st = SyncState::load(d.path()).unwrap().unwrap();
    assert_eq!(st.base_commit, f.head("main"));
    assert!(!st.files.contains_key("main.slint") && !st.files.contains_key("sub/deep.slint"));
}

#[test]
fn push_makes_one_commit_with_only_changes() {
    let f = seeded();
    let d = cloned(&f);
    let before_commits = f.commit_count();
    let base = f.head("main");
    write(d.path(), "main.slint", b"changed\n");
    write(d.path(), "new/file.slint", b"fresh\n");
    // management files must not be sent
    for m in [
        ".preview",
        ".settings",
        ".recovery/main.slint",
        "main.slint.bak",
        "x.local",
        ".last-open",
        ".github-token",
    ] {
        write(d.path(), m, b"private");
    }
    assert_eq!(local_changes(d.path()).unwrap().len(), 2);
    let r = push(&f, d.path(), "msg").unwrap();
    assert_eq!((r.added, r.modified, r.deleted), (1, 1, 0));
    assert_eq!(f.commit_count(), before_commits + 1);
    assert_eq!(f.head("main"), r.commit);
    assert_eq!(f.parent(&r.commit), Some(base));
    let l = f.listing("main");
    assert_eq!(l["ui/main.slint"], b"changed\n");
    assert_eq!(l["ui/new/file.slint"], b"fresh\n");
    assert_eq!(l["ui/sub/deep.slint"], b"deep\n");
    assert_eq!(l["README.md"], b"outside\n");
    let keys: Vec<&str> = l.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        vec![
            "README.md",
            "ui/.settings",
            "ui/img/pic.bin",
            "ui/main.slint",
            "ui/new/file.slint",
            "ui/old.bak",
            "ui/sub/deep.slint"
        ]
    );
    assert_eq!(l["ui/.settings"], b"remote-managed");
    assert_eq!(l["ui/old.bak"], b"bak");
    let st = SyncState::load(d.path()).unwrap().unwrap();
    assert_eq!(st.base_commit, r.commit);
    assert!(local_changes(d.path()).unwrap().is_empty());
    assert!(matches!(
        push(&f, d.path(), "again"),
        Err(SyncError::NothingToPush)
    ));
}

#[test]
fn push_deletes_missing_tracked_files() {
    let f = seeded();
    let d = cloned(&f);
    fs::remove_file(d.path().join("img/pic.bin")).unwrap();
    assert_eq!(
        local_changes(d.path()).unwrap(),
        vec![Change {
            path: "img/pic.bin".into(),
            kind: ChangeKind::Deleted
        }]
    );
    let r = push(&f, d.path(), "rm").unwrap();
    assert_eq!(r.deleted, 1);
    assert!(!f.listing("main").contains_key("ui/img/pic.bin"));
    assert!(
        !SyncState::load(d.path())
            .unwrap()
            .unwrap()
            .files
            .contains_key("img/pic.bin")
    );
}

#[test]
fn push_behind_remote_changes_nothing() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    let remote_head = f.commit("main", &[("ui/other.slint", Some(b"o"))]);
    let snap = snapshot(d.path());
    let commits = f.commit_count();
    assert!(matches!(
        push(&f, d.path(), "m"),
        Err(SyncError::BehindRemote)
    ));
    assert_eq!(snapshot(d.path()), snap);
    assert_eq!(f.head("main"), remote_head);
    assert_eq!(f.commit_count(), commits);
}

#[test]
fn push_loses_race_at_update_ref() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    f.0.lock().unwrap().race_on_update = true;
    let snap = snapshot(d.path());
    assert!(matches!(
        push(&f, d.path(), "m"),
        Err(SyncError::BehindRemote)
    ));
    assert_eq!(snapshot(d.path()), snap);
    assert!(
        !f.listing("main")
            .get("ui/main.slint")
            .is_some_and(|c| c == b"mine\n")
    );
}

#[test]
fn failure_at_any_step_leaves_pull_untouched() {
    for op in ["branch_head", "commit_tree", "tree_recursive", "blob"] {
        let f = seeded();
        let d = cloned(&f);
        write(d.path(), "sub/deep.slint", b"local edit\n");
        f.commit(
            "main",
            &[
                ("ui/main.slint", Some(b"new\n")),
                ("ui/z.slint", Some(b"z\n")),
                ("ui/img/pic.bin", None),
            ],
        );
        let snap = snapshot(d.path());
        f.fail_on(Some(op));
        assert!(pull(&f, d.path()).is_err(), "{op}");
        assert_eq!(
            snapshot(d.path()),
            snap,
            "files changed after failure at {op}"
        );
        f.fail_on(None);
        // and it works afterwards
        let r = pull(&f, d.path()).unwrap();
        assert_eq!(r.deleted, vec!["img/pic.bin"]);
    }
}

#[test]
fn failure_at_any_step_leaves_push_untouched() {
    for op in [
        "branch_head",
        "commit_tree",
        "create_blob",
        "create_tree",
        "create_commit",
        "update_ref",
    ] {
        let f = seeded();
        let d = cloned(&f);
        write(d.path(), "main.slint", b"mine\n");
        let snap = snapshot(d.path());
        let head = f.head("main");
        f.fail_on(Some(op));
        assert!(push(&f, d.path(), "m").is_err(), "{op}");
        assert_eq!(snapshot(d.path()), snap, "{op}");
        assert_eq!(f.head("main"), head, "{op}");
        f.fail_on(None);
        push(&f, d.path(), "m").unwrap();
    }
}

#[test]
fn corrupt_blob_is_rejected_without_writing() {
    let f = seeded();
    let d = cloned(&f);
    let c = f.commit("main", &[("ui/main.slint", Some(b"new\n"))]);
    // damage the stored blob content behind its sha
    {
        let mut i = f.0.lock().unwrap();
        let sha = git_blob_sha(b"new\n");
        i.blobs.insert(sha, b"tampered".to_vec());
    }
    let snap = snapshot(d.path());
    assert!(matches!(pull(&f, d.path()), Err(SyncError::Corrupt(_))));
    assert_eq!(snapshot(d.path()), snap);
    assert_ne!(SyncState::load(d.path()).unwrap().unwrap().base_commit, c);
}

#[test]
fn unlinked_project() {
    let d = tempfile::tempdir().unwrap();
    let f = seeded();
    assert!(matches!(pull(&f, d.path()), Err(SyncError::NotLinked)));
    assert!(matches!(local_changes(d.path()), Err(SyncError::NotLinked)));
    assert!(matches!(push(&f, d.path(), "m"), Err(SyncError::NotLinked)));
}

#[test]
fn keep_local_pushes_my_version_on_the_new_head() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    let c = f.commit(
        "main",
        &[
            ("ui/main.slint", Some(b"theirs\n")),
            ("ui/sub/deep.slint", Some(b"d2\n")),
        ],
    );
    assert_eq!(pull(&f, d.path()).unwrap().conflicts, vec!["main.slint"]);
    keep_local(&f, d.path(), "main.slint").unwrap();
    assert_eq!(read(d.path(), "main.slint"), b"mine\n");
    assert!(!d.path().join("main.slint.local").exists());
    let st = SyncState::load(d.path()).unwrap().unwrap();
    assert_eq!(st.base_commit, c);
    assert!(st.conflicts.is_empty() && st.pending_head.is_none());
    assert_eq!(
        local_changes(d.path()).unwrap(),
        vec![Change {
            path: "main.slint".into(),
            kind: ChangeKind::Modified
        }]
    );
    let r = push(&f, d.path(), "mine wins").unwrap();
    assert_eq!(f.parent(&r.commit), Some(c));
    let l = f.listing("main");
    assert_eq!(l["ui/main.slint"], b"mine\n");
    assert_eq!(l["ui/sub/deep.slint"], b"d2\n");
}

#[test]
fn mixed_keep_local_and_take_remote() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    write(d.path(), "sub/deep.slint", b"mine deep\n");
    let c = f.commit(
        "main",
        &[
            ("ui/main.slint", Some(b"t1\n")),
            ("ui/sub/deep.slint", Some(b"t2\n")),
        ],
    );
    let r = pull(&f, d.path()).unwrap();
    assert_eq!(r.conflicts.len(), 2);
    keep_local(&f, d.path(), "main.slint").unwrap();
    assert!(matches!(
        push(&f, d.path(), "m"),
        Err(SyncError::ConflictsPending)
    ));
    assert_eq!(
        SyncState::load(d.path()).unwrap().unwrap().conflicts,
        vec!["sub/deep.slint"]
    );
    take_remote(&f, d.path(), "sub/deep.slint").unwrap();
    assert_eq!(SyncState::load(d.path()).unwrap().unwrap().base_commit, c);
    assert_eq!(read(d.path(), "sub/deep.slint"), b"t2\n");
    assert_eq!(read(d.path(), "sub/deep.slint.local"), b"mine deep\n");
    let r = push(&f, d.path(), "m").unwrap();
    assert_eq!((r.added, r.modified, r.deleted), (0, 1, 0));
    let l = f.listing("main");
    assert_eq!(l["ui/main.slint"], b"mine\n");
    assert_eq!(l["ui/sub/deep.slint"], b"t2\n");
}

#[test]
fn take_remote_keeps_an_existing_local_copy() {
    let f = seeded();
    let d = cloned(&f);
    write(d.path(), "main.slint", b"mine\n");
    write(d.path(), "main.slint.local", b"older keep\n");
    f.commit("main", &[("ui/main.slint", Some(b"theirs\n"))]);
    pull(&f, d.path()).unwrap();
    take_remote(&f, d.path(), "main.slint").unwrap();
    assert_eq!(read(d.path(), "main.slint.local"), b"older keep\n");
    assert_eq!(read(d.path(), "main.slint.local.2"), b"mine\n");
    assert!(local_changes(d.path()).unwrap().is_empty());
}

#[test]
fn hostile_remote_entries_are_skipped() {
    let hostile: &[(&str, &str)] = &[
        ("../x", "100644"),
        ("/abs", "100644"),
        ("a/../../b", "100644"),
        ("a\\b", "100644"),
        ("link", "120000"),
        ("submod", "160000"),
        ("ok/fine.txt", "100644"),
    ];
    for folder in ["ui", ""] {
        let f = seeded();
        let prefix = if folder.is_empty() {
            String::new()
        } else {
            format!("{folder}/")
        };
        let paths: Vec<String> = hostile
            .iter()
            .map(|(p, _)| format!("{prefix}{p}"))
            .collect();
        let changes: Vec<(&str, Option<&[u8]>, &str)> = paths
            .iter()
            .zip(hostile)
            .map(|(p, (_, m))| (p.as_str(), Some(&b"evil"[..]), *m))
            .collect();
        f.commit_modes("main", &changes);
        let parent = tempfile::tempdir().unwrap();
        let before = snapshot(parent.path());
        let proj = parent.path().join("proj");
        let st = clone_into(&f, REPO, "main", folder, &proj).unwrap();
        let mut got: Vec<&str> = st.files.keys().map(String::as_str).collect();
        got.sort();
        assert!(got.contains(&"ok/fine.txt"));
        for bad in ["../x", "/abs", "a/../../b", "a\\b", "link", "submod"] {
            assert!(!got.contains(&bad), "{bad} imported (folder {folder:?})");
        }
        let _ = before;
        let all = snapshot(parent.path());
        assert!(
            all.keys().all(|k| k.starts_with("proj/")),
            "{:?}",
            all.keys().collect::<Vec<_>>()
        );
        assert!(!proj.join("link").exists() && !proj.join("submod").exists());

        // pull path: hostile entries arrive in a later commit
        let f2 = seeded();
        let proj2 = parent.path().join("proj2");
        clone_into(&f2, REPO, "main", folder, &proj2).unwrap();
        f2.commit_modes("main", &changes);
        pull(&f2, &proj2).unwrap();
        let all = snapshot(parent.path());
        assert!(
            all.keys()
                .all(|k| k.starts_with("proj/") || k.starts_with("proj2/"))
        );
        assert!(!proj2.join("link").exists() && !proj2.join("submod").exists());
        assert_eq!(read(&proj2, "ok/fine.txt"), b"evil");
    }
}

#[test]
fn pull_when_folder_is_gone_deletes_nothing() {
    let f = seeded();
    let d = cloned(&f);
    f.commit(
        "main",
        &[
            ("ui/main.slint", None),
            ("ui/sub/deep.slint", None),
            ("ui/img/pic.bin", None),
            ("ui/.settings", None),
            ("ui/old.bak", None),
        ],
    );
    let snap = snapshot(d.path());
    assert!(matches!(pull(&f, d.path()), Err(SyncError::EmptyFolder)));
    assert_eq!(snapshot(d.path()), snap);
}
