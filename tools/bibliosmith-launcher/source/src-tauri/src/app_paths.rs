use serde::{Deserialize, Serialize};
use std::{
    env,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::OnceLock,
};
use tauri::{path::BaseDirectory, Manager};

pub(crate) const WORKSPACE_MARKER: &str = ".bibliosmith-workspace.json";
const WORKSPACE_SCHEMA: &str = "bibliosmith-workspace-v1";

static APP_PATHS: OnceLock<AppPaths> = OnceLock::new();

#[derive(Debug, Clone)]
pub(crate) struct AppPaths {
    resource_root: PathBuf,
    support_root: PathBuf,
    cache_root: PathBuf,
    documents_root: PathBuf,
}

impl AppPaths {
    pub(crate) fn from_roots(
        resource_root: PathBuf,
        support_root: PathBuf,
        cache_root: PathBuf,
        documents_root: PathBuf,
    ) -> Self {
        Self {
            resource_root,
            support_root,
            cache_root,
            documents_root,
        }
    }

    pub(crate) fn resource_root(&self) -> PathBuf {
        self.resource_root.clone()
    }

    pub(crate) fn support_root(&self) -> PathBuf {
        self.support_root.clone()
    }

    pub(crate) fn cache_root(&self) -> PathBuf {
        self.cache_root.clone()
    }

    pub(crate) fn runtime_bin_root(&self) -> PathBuf {
        let executable_dir = env::current_exe()
            .ok()
            .and_then(|path| path.parent().map(Path::to_path_buf));
        if !cfg!(debug_assertions) {
            return executable_dir.unwrap_or_else(|| self.resource_root.join("bin"));
        }
        executable_dir
            .filter(|directory| {
                ["node", "uv"].iter().any(|name| {
                    directory
                        .join(if cfg!(target_os = "windows") {
                            format!("{name}.exe")
                        } else {
                            (*name).to_string()
                        })
                        .is_file()
                })
            })
            .unwrap_or_else(|| self.resource_root.join("bin"))
    }

    pub(crate) fn recommended_workspace_root(&self) -> PathBuf {
        self.documents_root.join("BiblioSmith")
    }
}

pub(crate) fn initialize(app: &tauri::App) -> Result<(), String> {
    let paths = system_paths(app)?;
    fs::create_dir_all(paths.support_root()).map_err(|error| {
        format!(
            "无法创建 BiblioSmith Application Support 目录 {}：{error}",
            paths.support_root().display()
        )
    })?;
    fs::create_dir_all(paths.cache_root()).map_err(|error| {
        format!(
            "无法创建 BiblioSmith Cache 目录 {}：{error}",
            paths.cache_root().display()
        )
    })?;
    APP_PATHS
        .set(paths)
        .map_err(|_| "BiblioSmith 路径已经初始化。".to_string())
}

pub(crate) fn current() -> Result<AppPaths, String> {
    if let Some(paths) = APP_PATHS.get() {
        return Ok(paths.clone());
    }
    development_paths()
}

fn system_paths(app: &tauri::App) -> Result<AppPaths, String> {
    let resource_root = if cfg!(debug_assertions) {
        development_resource_root()?
    } else {
        app.path()
            .resolve("bibliosmith-runtime", BaseDirectory::Resource)
            .map_err(|error| format!("无法定位 BiblioSmith App 资源：{error}"))?
    };
    let support_base = dirs::data_local_dir()
        .or_else(dirs::config_local_dir)
        .ok_or_else(|| "无法定位系统 Application Support 目录。".to_string())?;
    let cache_base = dirs::cache_dir().ok_or_else(|| "无法定位系统 Cache 目录。".to_string())?;
    let documents_root =
        dirs::document_dir().ok_or_else(|| "无法定位用户 Documents 目录。".to_string())?;
    Ok(AppPaths::from_roots(
        resource_root,
        app_owned_root(&support_base),
        app_owned_root(&cache_base),
        documents_root,
    ))
}

fn development_paths() -> Result<AppPaths, String> {
    if !cfg!(debug_assertions) {
        return Err("BiblioSmith App 路径尚未初始化。".into());
    }
    let support_base = dirs::data_local_dir()
        .or_else(dirs::config_local_dir)
        .ok_or_else(|| "无法定位系统 Application Support 目录。".to_string())?;
    let cache_base = dirs::cache_dir().ok_or_else(|| "无法定位系统 Cache 目录。".to_string())?;
    let documents_root =
        dirs::document_dir().ok_or_else(|| "无法定位用户 Documents 目录。".to_string())?;
    Ok(AppPaths::from_roots(
        development_resource_root()?,
        app_owned_root(&support_base),
        app_owned_root(&cache_base),
        documents_root,
    ))
}

fn app_owned_root(base: &Path) -> PathBuf {
    base.join("BiblioSmith").join(if cfg!(dev) {
        "launcher-dev"
    } else {
        "launcher"
    })
}

fn development_resource_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .find(|candidate| {
            candidate.join("pyproject.toml").is_file()
                && candidate.join("packages/ocr/pyproject.toml").is_file()
                && candidate
                    .join("tools/bibliosmith-launcher/source/scripts")
                    .is_dir()
        })
        .map(Path::to_path_buf)
        .ok_or_else(|| "无法定位开发环境的 BiblioSmith 只读资源根目录。".to_string())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum WorkspaceStatus {
    Missing,
    Empty,
    Ready,
    Occupied,
}

#[derive(Debug, Deserialize, Serialize)]
struct WorkspaceMarker {
    schema: String,
}

fn workspace_marker_is_valid(path: &Path) -> bool {
    fs::read_to_string(path)
        .ok()
        .and_then(|text| serde_json::from_str::<WorkspaceMarker>(&text).ok())
        .is_some_and(|marker| marker.schema == WORKSPACE_SCHEMA)
}

pub(crate) fn workspace_status(workspace_root: &Path) -> WorkspaceStatus {
    if !workspace_root.exists() {
        return WorkspaceStatus::Missing;
    }
    if !workspace_root.is_dir() {
        return WorkspaceStatus::Occupied;
    }
    let mut entries = match fs::read_dir(workspace_root) {
        Ok(entries) => entries,
        Err(_) => return WorkspaceStatus::Occupied,
    };
    if entries.next().is_none() {
        return WorkspaceStatus::Empty;
    }
    let marker = workspace_root.join(WORKSPACE_MARKER);
    if workspace_marker_is_valid(&marker) {
        if workspace_root.join("books/local/zh-Hans").is_dir() {
            WorkspaceStatus::Ready
        } else {
            // A valid marker is an App-owned, resumable partial creation. It is
            // not an occupied user directory, so retrying startup can finish
            // the contract after a disk or permission failure.
            WorkspaceStatus::Empty
        }
    } else {
        let pending_marker = workspace_root.join(format!("{WORKSPACE_MARKER}.creating"));
        let pending_is_only_entry = workspace_marker_is_valid(&pending_marker)
            && fs::read_dir(workspace_root).ok().is_some_and(|entries| {
                entries
                    .filter_map(Result::ok)
                    .all(|entry| entry.path() == pending_marker)
            });
        if pending_is_only_entry {
            WorkspaceStatus::Empty
        } else {
            WorkspaceStatus::Occupied
        }
    }
}

pub(crate) fn create_workspace(workspace_root: &Path) -> Result<(), String> {
    if !workspace_root.is_absolute() {
        return Err("BiblioSmith 工作区必须使用绝对路径。".into());
    }
    reject_symlink_path(workspace_root)?;
    match workspace_status(workspace_root) {
        WorkspaceStatus::Ready => return Ok(()),
        WorkspaceStatus::Missing | WorkspaceStatus::Empty => {}
        WorkspaceStatus::Occupied => {
            return Err(format!(
                "目标目录已有其他文件，BiblioSmith 不会覆盖或混入其中：{}",
                workspace_root.display()
            ));
        }
    }
    let created_root = if workspace_root.exists() {
        false
    } else {
        let parent = workspace_root
            .parent()
            .ok_or_else(|| "BiblioSmith 工作区缺少父目录。".to_string())?;
        fs::create_dir_all(parent)
            .map_err(|error| format!("无法创建 BiblioSmith 工作区父目录：{error}"))?;
        reject_symlink_path(parent)?;
        fs::create_dir(workspace_root)
            .map_err(|error| format!("无法创建 BiblioSmith 工作区：{error}"))?;
        true
    };
    reject_symlink_path(workspace_root)?;
    let marker = WorkspaceMarker {
        schema: WORKSPACE_SCHEMA.into(),
    };
    let marker_text = serde_json::to_string_pretty(&marker).map_err(|error| error.to_string())?;
    let marker_path = workspace_root.join(WORKSPACE_MARKER);
    if !marker_path.is_file() {
        let pending_marker = workspace_root.join(format!("{WORKSPACE_MARKER}.creating"));
        let write_result = (|| -> Result<(), String> {
            if !workspace_marker_is_valid(&pending_marker) {
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&pending_marker)
                    .map_err(|error| format!("无法创建 BiblioSmith 工作区标记：{error}"))?;
                file.write_all(format!("{marker_text}\n").as_bytes())
                    .and_then(|_| file.sync_all())
                    .map_err(|error| format!("无法写入 BiblioSmith 工作区标记：{error}"))?;
            }
            let has_unexpected_entry = fs::read_dir(workspace_root)
                .map_err(|error| error.to_string())?
                .filter_map(Result::ok)
                .any(|entry| entry.path() != pending_marker);
            if has_unexpected_entry {
                return Err("目标目录在创建过程中出现了其他文件，BiblioSmith 已停止写入。".into());
            }
            fs::rename(&pending_marker, &marker_path)
                .map_err(|error| format!("无法确认 BiblioSmith 工作区标记：{error}"))?;
            #[cfg(unix)]
            fs::File::open(workspace_root)
                .and_then(|directory| directory.sync_all())
                .map_err(|error| format!("无法同步 BiblioSmith 工作区标记：{error}"))?;
            Ok(())
        })();
        if let Err(error) = write_result {
            let _ = fs::remove_file(&pending_marker);
            if created_root {
                let _ = fs::remove_dir(workspace_root);
            }
            return Err(error);
        }
    }
    fs::create_dir_all(workspace_root.join("books/local/zh-Hans"))
        .map_err(|error| format!("无法创建 BiblioSmith 工作区：{error}"))
}

fn reject_symlink_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(format!(
            "BiblioSmith 工作区路径不能是符号链接：{}",
            path.display()
        )),
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("无法检查 BiblioSmith 工作区路径：{error}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_paths_keep_resources_support_cache_and_workspace_separate() {
        let temp = tempfile::tempdir().expect("tempdir");
        let resources = temp
            .path()
            .join("App.app/Contents/Resources/bibliosmith-runtime");
        let support = temp
            .path()
            .join("Library/Application Support/BiblioSmith/launcher");
        let cache = temp.path().join("Library/Caches/BiblioSmith/launcher");
        let documents = temp.path().join("Documents");

        let paths = AppPaths::from_roots(
            resources.clone(),
            support.clone(),
            cache.clone(),
            documents.clone(),
        );

        assert_eq!(paths.resource_root(), resources);
        assert_eq!(paths.support_root(), support);
        assert_eq!(paths.cache_root(), cache);
        assert_eq!(
            paths.recommended_workspace_root(),
            documents.join("BiblioSmith")
        );
    }

    #[test]
    fn debug_builds_keep_mutable_app_state_out_of_the_release_directory() {
        let base = Path::new("Library/Application Support");

        assert_eq!(
            app_owned_root(base),
            base.join("BiblioSmith").join("launcher-dev")
        );
    }

    #[test]
    fn create_workspace_builds_the_user_owned_contract_without_a_git_repository() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("Documents/BiblioSmith");

        create_workspace(&workspace).expect("create workspace");

        assert_eq!(workspace_status(&workspace), WorkspaceStatus::Ready);
        assert!(workspace.join(WORKSPACE_MARKER).is_file());
        assert!(workspace.join("books/local/zh-Hans").is_dir());
        assert!(!workspace.join(".git").exists());
        assert!(!workspace.join("tools").exists());
    }

    #[test]
    fn create_workspace_refuses_to_mix_with_an_occupied_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("Documents/BiblioSmith");
        std::fs::create_dir_all(&workspace).expect("create occupied directory");
        std::fs::write(workspace.join("personal.txt"), "keep me").expect("write personal file");

        let error = create_workspace(&workspace).expect_err("occupied directory must be rejected");

        assert!(error.contains("已有其他文件"));
        assert_eq!(
            std::fs::read_to_string(workspace.join("personal.txt")).expect("personal file"),
            "keep me"
        );
        assert!(!workspace.join(WORKSPACE_MARKER).exists());
    }

    #[test]
    fn create_workspace_rejects_relative_paths_from_untrusted_callers() {
        let error = create_workspace(Path::new("Documents/BiblioSmith"))
            .expect_err("relative renderer path must be rejected");

        assert!(error.contains("绝对路径"));
    }

    #[cfg(unix)]
    #[test]
    fn create_workspace_rejects_symlink_targets() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().expect("tempdir");
        let real = temp.path().join("real");
        let alias = temp.path().join("alias");
        fs::create_dir(&real).expect("real directory");
        symlink(&real, &alias).expect("symlink");

        let error = create_workspace(&alias).expect_err("symlink target must be rejected");

        assert!(error.contains("符号链接"));
        assert_eq!(workspace_status(&real), WorkspaceStatus::Empty);
    }

    #[test]
    fn valid_partial_workspace_creation_is_resumable() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("BiblioSmith");
        fs::create_dir(&workspace).expect("workspace");
        fs::write(
            workspace.join(WORKSPACE_MARKER),
            "{\"schema\":\"bibliosmith-workspace-v1\"}\n",
        )
        .expect("marker");

        assert_eq!(workspace_status(&workspace), WorkspaceStatus::Empty);
        create_workspace(&workspace).expect("resume workspace");
        assert_eq!(workspace_status(&workspace), WorkspaceStatus::Ready);
    }

    #[test]
    fn pending_workspace_marker_is_resumable_after_a_crash() {
        let temp = tempfile::tempdir().expect("tempdir");
        let workspace = temp.path().join("BiblioSmith");
        fs::create_dir(&workspace).expect("workspace");
        fs::write(
            workspace.join(format!("{WORKSPACE_MARKER}.creating")),
            "{\"schema\":\"bibliosmith-workspace-v1\"}\n",
        )
        .expect("pending marker");

        assert_eq!(workspace_status(&workspace), WorkspaceStatus::Empty);
        create_workspace(&workspace).expect("resume pending marker");
        assert_eq!(workspace_status(&workspace), WorkspaceStatus::Ready);
        assert!(!workspace
            .join(format!("{WORKSPACE_MARKER}.creating"))
            .exists());
    }
}
