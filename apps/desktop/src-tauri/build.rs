fn main() {
    prune_stale_workspace_artifacts();
    tauri_build::build();
}

fn prune_stale_workspace_artifacts() {
    let out_dir = match std::env::var_os("OUT_DIR") {
        Some(dir) => std::path::PathBuf::from(dir),
        None => return,
    };

    let mut profile_dir = None;
    let mut current = out_dir.as_path();
    while let Some(parent) = current.parent() {
        let name = parent.file_name().and_then(|n| n.to_str());
        if name == Some("debug") || name == Some("release") {
            profile_dir = Some(parent.to_path_buf());
            break;
        }
        current = parent;
    }

    let profile_dir = match profile_dir {
        Some(dir) => dir,
        None => return,
    };

    let build_dir = profile_dir.join("build");
    if let Ok(entries) = std::fs::read_dir(&build_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                prune_crate_build_hashes(&path, &out_dir);
            }
        }
    }

    let incremental_dir = profile_dir.join("incremental");
    prune_incremental(&incremental_dir);
}

fn prune_crate_build_hashes(crate_build_dir: &std::path::Path, out_dir: &std::path::Path) {
    let entries = match std::fs::read_dir(crate_build_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let min_age = std::time::Duration::from_secs(24 * 3600);
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if out_dir.starts_with(&path) {
                continue;
            }
            if let Ok(meta) = path.metadata()
                && let Ok(modified) = meta.modified()
            {
                dirs.push((path, modified));
            }
        }
    }

    dirs.sort_by_key(|a| std::cmp::Reverse(a.1));

    for (path, modified) in dirs.into_iter().skip(6) {
        if modified.elapsed().is_ok_and(|age| age > min_age) {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}

fn prune_incremental(incremental_dir: &std::path::Path) {
    let entries = match std::fs::read_dir(incremental_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    let min_age = std::time::Duration::from_secs(24 * 3600);
    let mut dirs = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && let Some(name) = path.file_name().and_then(|n| n.to_str())
            && name.starts_with("halcyon_")
            && let Ok(meta) = path.metadata()
            && let Ok(modified) = meta.modified()
        {
            dirs.push((path, modified));
        }
    }

    dirs.sort_by_key(|a| std::cmp::Reverse(a.1));

    for (path, modified) in dirs.into_iter().skip(8) {
        if modified.elapsed().is_ok_and(|age| age > min_age) {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
