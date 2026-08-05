use chrono::Local;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::{
    any::Any,
    env, fs,
    io::{self, Read, Write},
    net::{IpAddr, TcpStream, ToSocketAddrs},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Output, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use tauri::{
    menu::MenuBuilder,
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, WindowEvent,
};

mod app_paths;
mod book_pipeline;
mod embedding_settings;
mod model_settings;
mod ocr_settings;
mod zotero_settings;

const BIBLIOSMITH_PYTHON_ENV: &str = "BIBLIOSMITH_PYTHON";
const BIBLIOSMITH_JAVA_ENV: &str = "BIBLIOSMITH_JAVA";
const RUNTIME_PROGRESS_EVENT: &str = "runtime-install-progress";
const TRAY_SHOW_ID: &str = "tray_show";
const TRAY_HIDE_ID: &str = "tray_hide";
const TRAY_QUIT_ID: &str = "tray_quit";
const LAUNCHER_LOG_MAX_BYTES: u64 = 5 * 1024 * 1024;
const LAUNCHER_LOG_BACKUP_COUNT: usize = 0;
const LAUNCHER_LOG_LEGACY_EXPORT_BACKUP_SCAN_COUNT: usize = 5;
const PROXY_TEST_TIMEOUT_SECONDS: u64 = 8;
const PROXY_PORT_PROBE_TIMEOUT_MS: u64 = 260;
const GITHUB_CONNECTIVITY_TEST_URL: &str = "https://api.github.com/";
const PYTHON_RUNTIME_VERSION: &str = "3.12.10";
const PYTHON_RUNTIME_DIR_NAME: &str = "python-3.12.10-embed-amd64";
const PYTHON_RUNTIME_ARCHIVE: &str = "python-3.12.10-embed-amd64.zip";
const PYTHON_RUNTIME_SHA256: &str =
    "4ACBED6DD1C744B0376E3B1CF57CE906F9DC9E95E68824584C8099A63025A3C3";
const PYTHON_RUNTIME_SIZE_BYTES: u64 = 11_133_606;
const PYTHON_RUNTIME_URLS: &[&str] = &[
    "https://www.python.org/ftp/python/3.12.10/python-3.12.10-embed-amd64.zip",
    "https://mirrors.huaweicloud.com/python/3.12.10/python-3.12.10-embed-amd64.zip",
    "https://registry.npmmirror.com/-/binary/python/3.12.10/python-3.12.10-embed-amd64.zip",
];
const JAVA_RUNTIME_VERSION: &str = "17.0.19";
const JAVA_RUNTIME_SHA256S: [&str; 3] = [
    "D6D0802E9BB5DA42A61E4891463CDE880F00A7BF5FE2BD41A4FF9260E52C4EBB",
    "6A2A6998DCCD031A3AA4F10138152B8B4D32859959226FE1FF2BDB1995B5B23B",
    "A9819DBC00814A849723608D73F8CB8FAE87D5CEF0B322B87277BF9DBED35420",
];
#[cfg(target_os = "windows")]
const JAVA_RUNTIME_DIR_NAME: &str = "zulu17.66.19-ca-jre17.0.19-win_x64";
#[cfg(target_os = "windows")]
const JAVA_RUNTIME_ARCHIVE: &str = "zulu17.66.19-ca-jre17.0.19-win_x64.zip";
#[cfg(target_os = "windows")]
const JAVA_RUNTIME_SHA256: &str = JAVA_RUNTIME_SHA256S[0];
#[cfg(target_os = "windows")]
const JAVA_RUNTIME_SIZE_BYTES: u64 = 44_097_076;
#[cfg(target_os = "windows")]
const JAVA_RUNTIME_URLS: &[&str] = &[
    "https://cdn.azul.com/zulu/bin/zulu17.66.19-ca-jre17.0.19-win_x64.zip",
    "https://static.azul.com/zulu/bin/zulu17.66.19-ca-jre17.0.19-win_x64.zip",
];
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JAVA_RUNTIME_DIR_NAME: &str = "zulu17.66.19-ca-jre17.0.19-macosx_aarch64";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JAVA_RUNTIME_ARCHIVE: &str = "zulu17.66.19-ca-jre17.0.19-macosx_aarch64.zip";
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JAVA_RUNTIME_SHA256: &str = JAVA_RUNTIME_SHA256S[1];
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JAVA_RUNTIME_SIZE_BYTES: u64 = 43_270_182;
#[cfg(all(target_os = "macos", target_arch = "aarch64"))]
const JAVA_RUNTIME_URLS: &[&str] = &[
    "https://cdn.azul.com/zulu/bin/zulu17.66.19-ca-jre17.0.19-macosx_aarch64.zip",
    "https://static.azul.com/zulu/bin/zulu17.66.19-ca-jre17.0.19-macosx_aarch64.zip",
];
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JAVA_RUNTIME_DIR_NAME: &str = "zulu17.66.19-ca-jre17.0.19-macosx_x64";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JAVA_RUNTIME_ARCHIVE: &str = "zulu17.66.19-ca-jre17.0.19-macosx_x64.zip";
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JAVA_RUNTIME_SHA256: &str = JAVA_RUNTIME_SHA256S[2];
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JAVA_RUNTIME_SIZE_BYTES: u64 = 44_287_760;
#[cfg(all(target_os = "macos", target_arch = "x86_64"))]
const JAVA_RUNTIME_URLS: &[&str] = &[
    "https://cdn.azul.com/zulu/bin/zulu17.66.19-ca-jre17.0.19-macosx_x64.zip",
    "https://static.azul.com/zulu/bin/zulu17.66.19-ca-jre17.0.19-macosx_x64.zip",
];
const RUNTIME_HTTP_CONNECT_TIMEOUT_SECONDS: u64 = 12;
const RUNTIME_HTTP_REQUEST_TIMEOUT_SECONDS: u64 = 180;
const RUNTIME_PROBE_TIMEOUT_SECONDS: u64 = 6;
static RUNTIME_PREPARE_RUNNING: AtomicBool = AtomicBool::new(false);
static LAUNCHER_CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn launcher_log_path() -> Result<PathBuf, String> {
    Ok(app_paths::current()?
        .support_root()
        .join("logs")
        .join("bibliosmith-launcher.log"))
}

fn append_launcher_log(level: &str, message: impl AsRef<str>) {
    let Ok(path) = launcher_log_path() else {
        return;
    };
    let _ = append_launcher_log_to_path(
        &path,
        launcher_logging_enabled(),
        LAUNCHER_LOG_MAX_BYTES,
        LAUNCHER_LOG_BACKUP_COUNT,
        level,
        message.as_ref(),
    );
}

fn append_launcher_log_to_path(
    path: &Path,
    enabled: bool,
    max_bytes: u64,
    backup_count: usize,
    level: &str,
    message: &str,
) -> Result<(), String> {
    if !enabled {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f %:z");
    let line = format!("[{timestamp}] [{level}] {message}\n");
    let existing_bytes = fs::metadata(path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if max_bytes > 0 && existing_bytes > 0 && existing_bytes + line.len() as u64 > max_bytes {
        rotate_launcher_log(path, backup_count)?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|err| err.to_string())?;
    file.write_all(line.as_bytes())
        .map_err(|err| err.to_string())
}

fn rotate_launcher_log(path: &Path, backup_count: usize) -> Result<(), String> {
    if backup_count == 0 {
        if path.exists() {
            fs::remove_file(path).map_err(|err| err.to_string())?;
        }
        return Ok(());
    }
    let oldest = rotated_log_path(path, backup_count);
    if oldest.exists() {
        fs::remove_file(&oldest).map_err(|err| err.to_string())?;
    }
    for index in (1..backup_count).rev() {
        let source = rotated_log_path(path, index);
        if source.exists() {
            fs::rename(&source, rotated_log_path(path, index + 1))
                .map_err(|err| err.to_string())?;
        }
    }
    if path.exists() {
        fs::rename(path, rotated_log_path(path, 1)).map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn rotated_log_path(path: &Path, index: usize) -> PathBuf {
    if let Some(extension) = path.extension().and_then(|value| value.to_str()) {
        path.with_extension(format!("{extension}.{index}"))
    } else {
        let file_name = path
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("bibliosmith-launcher");
        path.with_file_name(format!("{file_name}.{index}"))
    }
}

fn launcher_logging_enabled() -> bool {
    read_launcher_config()
        .as_ref()
        .map(diagnostic_logging_enabled_from_config)
        .unwrap_or(true)
}

fn install_panic_log_hook() {
    std::panic::set_hook(Box::new(|info| {
        append_launcher_log("ERROR", format!("panic: {info}"));
    }));
}

fn panic_payload_to_string(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "unknown panic payload".into()
    }
}

fn diagnostic_logging_enabled_from_config(config: &LauncherConfig) -> bool {
    config.save_logs.unwrap_or(true)
}

pub(crate) async fn run_blocking<T, F>(work: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    tauri::async_runtime::spawn_blocking(move || {
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
            .map_err(|payload| {
                format!(
                    "后台任务执行异常：{}",
                    panic_payload_to_string(payload.as_ref())
                )
            })
            .and_then(|result| result)
    })
    .await
    .map_err(|err| format!("后台任务执行失败：{err}"))?
}

struct RuntimePrepareGuard;

impl RuntimePrepareGuard {
    fn try_acquire() -> Result<Self, String> {
        RUNTIME_PREPARE_RUNNING
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .map(|_| RuntimePrepareGuard)
            .map_err(|_| "Python / Java 运行环境正在后台准备，请等待当前任务完成。".to_string())
    }
}

impl Drop for RuntimePrepareGuard {
    fn drop(&mut self) {
        RUNTIME_PREPARE_RUNNING.store(false, Ordering::Release);
    }
}

#[derive(Clone)]
struct RuntimeProgressEmitter {
    app: tauri::AppHandle,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkspaceState {
    workspace_root: String,
    recommended_workspace_root: String,
    workspace_ready: bool,
    workspace_status: app_paths::WorkspaceStatus,
    proxy_configured: bool,
    platform: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ActionResult {
    ok: bool,
    message: String,
    requires_download: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
struct LauncherConfig {
    workspace_root: Option<String>,
    save_logs: Option<bool>,
    proxy: Option<NetworkProxySettings>,
    active_model: Option<model_settings::ActiveModel>,
    qwen_workspace_id: Option<String>,
    qwen_web_search_enabled: Option<bool>,
}

pub(crate) fn read_active_model() -> Option<model_settings::ActiveModel> {
    read_launcher_config()?.active_model
}

pub(crate) fn write_active_model(
    active_model: Option<model_settings::ActiveModel>,
) -> Result<(), String> {
    update_launcher_config(|config| config.active_model = active_model)
}

pub(crate) fn read_qwen_workspace_id() -> Option<String> {
    read_launcher_config()?.qwen_workspace_id
}

pub(crate) fn read_qwen_web_search_enabled() -> bool {
    read_launcher_config()
        .and_then(|config| config.qwen_web_search_enabled)
        .unwrap_or(false)
}

pub(crate) fn write_qwen_settings(
    workspace_id: Option<String>,
    web_search_enabled: bool,
) -> Result<(), String> {
    update_launcher_config(|config| {
        config.qwen_workspace_id = workspace_id;
        config.qwen_web_search_enabled = Some(web_search_enabled);
    })
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct NetworkProxySettings {
    enabled: bool,
    scheme: String,
    host: String,
    port: Option<u16>,
}

impl Default for NetworkProxySettings {
    fn default() -> Self {
        Self {
            enabled: false,
            scheme: "http".into(),
            host: "127.0.0.1".into(),
            port: Some(7890),
        }
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyTestResult {
    ok: bool,
    message: String,
    elapsed_ms: Option<u128>,
    http_version: Option<String>,
    target_url: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ProxyAutoDetectResult {
    detected: bool,
    proxy: Option<NetworkProxySettings>,
    test: Option<ProxyTestResult>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeToolStatus {
    ready: bool,
    private_ready: bool,
    version: String,
    source: Option<String>,
    path: Option<String>,
    message: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RuntimeStatus {
    ready: bool,
    private_ready: bool,
    running: bool,
    runtime_root: String,
    python: RuntimeToolStatus,
    java: RuntimeToolStatus,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticLogSettings {
    save_logs: bool,
    log_dir: String,
    log_file: String,
    max_bytes: u64,
    backup_count: usize,
    max_total_bytes: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticExportContext {
    generated_at: String,
    launcher_version: String,
    os: String,
    arch: String,
    workspace_root: String,
    workspace_status: app_paths::WorkspaceStatus,
    save_logs: bool,
    log_dir: String,
    log_max_bytes: u64,
    log_backup_count: usize,
    proxy_configured: bool,
}

#[derive(Debug, Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct DownloadProgress {
    percent: f64,
    downloaded_bytes: u64,
    total_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    state: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RuntimeKind {
    Python,
    Java,
}

impl RuntimeKind {
    fn label(self) -> &'static str {
        match self {
            RuntimeKind::Python => "Python",
            RuntimeKind::Java => "Java",
        }
    }

    fn dir_name(self) -> &'static str {
        match self {
            RuntimeKind::Python => "python",
            RuntimeKind::Java => "java",
        }
    }

    fn env_name(self) -> &'static str {
        match self {
            RuntimeKind::Python => BIBLIOSMITH_PYTHON_ENV,
            RuntimeKind::Java => BIBLIOSMITH_JAVA_ENV,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct RuntimePackage {
    kind: RuntimeKind,
    version: &'static str,
    install_dir_name: &'static str,
    archive_name: &'static str,
    sha256: &'static str,
    size_bytes: u64,
    urls: &'static [&'static str],
}

#[tauri::command]
async fn get_workspace_state() -> Result<WorkspaceState, String> {
    run_blocking(collect_workspace_state).await
}

fn collect_workspace_state() -> Result<WorkspaceState, String> {
    let paths = app_paths::current()?;
    let config = read_launcher_config();
    Ok(collect_workspace_state_from(&paths, config.as_ref()))
}

fn collect_workspace_state_from(
    paths: &app_paths::AppPaths,
    config: Option<&LauncherConfig>,
) -> WorkspaceState {
    let recommended = paths.recommended_workspace_root();
    let workspace_root =
        configured_workspace_root_from_config(config).unwrap_or_else(|| recommended.clone());
    let workspace_status = app_paths::workspace_status(&workspace_root);
    WorkspaceState {
        workspace_root: display_path(&workspace_root),
        recommended_workspace_root: display_path(&recommended),
        workspace_ready: workspace_status == app_paths::WorkspaceStatus::Ready,
        workspace_status,
        proxy_configured: is_proxy_configured(),
        platform: format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    }
}

fn initialize_workspace_at(workspace_root: &Path) -> Result<WorkspaceState, String> {
    app_paths::create_workspace(workspace_root)?;
    write_workspace_root(workspace_root)?;
    collect_workspace_state()
}

#[tauri::command]
fn create_recommended_workspace() -> Result<WorkspaceState, String> {
    let workspace_root = app_paths::current()?.recommended_workspace_root();
    initialize_workspace_at(&workspace_root)
}

#[tauri::command]
fn choose_and_create_workspace() -> Result<Option<WorkspaceState>, String> {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("选择 BiblioSmith 书库位置")
        .pick_folder()
    else {
        return Ok(None);
    };
    initialize_workspace_at(&folder).map(Some)
}

#[tauri::command]
fn get_diagnostic_log_settings() -> Result<DiagnosticLogSettings, String> {
    diagnostic_log_settings()
}

#[tauri::command]
fn set_save_logs_enabled(save_logs: bool) -> Result<DiagnosticLogSettings, String> {
    if !save_logs {
        append_launcher_log("INFO", "diagnostic logging disabled by user");
    }
    write_save_logs_config(save_logs)?;
    if save_logs {
        append_launcher_log("INFO", "diagnostic logging enabled by user");
    }
    diagnostic_log_settings()
}

#[tauri::command]
fn get_proxy_settings() -> Result<NetworkProxySettings, String> {
    Ok(configured_proxy_settings())
}

#[tauri::command]
fn save_proxy_settings(proxy: NetworkProxySettings) -> Result<NetworkProxySettings, String> {
    write_proxy_config(proxy)
}

#[tauri::command]
async fn test_proxy_settings(proxy: NetworkProxySettings) -> Result<ProxyTestResult, String> {
    let proxy_url = proxy_url_from_settings(&proxy)?;
    let Some(proxy_url) = proxy_url else {
        return Ok(ProxyTestResult {
            ok: false,
            message: "请先启用代理并填写 IP/端口。".into(),
            elapsed_ms: None,
            http_version: None,
            target_url: GITHUB_CONNECTIVITY_TEST_URL.into(),
        });
    };

    match test_github_connectivity_via_proxy(&proxy_url, false).await {
        Ok(result) => Ok(result),
        Err(auto_error) => {
            append_launcher_log(
                "WARN",
                format!("proxy automatic HTTP test failed, retrying HTTP/1.1: {auto_error}"),
            );
            match test_github_connectivity_via_proxy(&proxy_url, true).await {
                Ok(result) => Ok(result),
                Err(retry_error) => Ok(proxy_test_failure_result(format!(
                    "代理测试失败。自动 HTTP：{auto_error}；HTTP/1.1 重试：{retry_error}"
                ))),
            }
        }
    }
}

#[tauri::command]
async fn auto_detect_proxy_settings(force: Option<bool>) -> Result<ProxyAutoDetectResult, String> {
    let force = force.unwrap_or(false);
    let current = configured_proxy_settings();
    if current.enabled && !force {
        return Ok(ProxyAutoDetectResult {
            detected: true,
            proxy: Some(current),
            test: None,
            message: "已启用手动代理设置，自动识别不会覆盖。".into(),
        });
    }

    let mut last_error = String::new();
    for candidate in proxy_detection_candidates_with_current(&current) {
        if !proxy_candidate_port_open_quick(&candidate) {
            last_error = proxy_candidate_label(&candidate, "本机端口未监听");
            append_launcher_log(
                "DEBUG",
                format!("skip proxy auto detect candidate: {last_error}"),
            );
            continue;
        }
        let saved = write_proxy_config(candidate)?;
        append_launcher_log(
            "INFO",
            format!(
                "auto detected proxy settings scheme={} host={} port={:?}",
                saved.scheme, saved.host, saved.port
            ),
        );
        return Ok(ProxyAutoDetectResult {
            detected: true,
            proxy: Some(saved),
            test: None,
            message: "识别成功，请点击“测试连接”。".into(),
        });
    }

    Ok(ProxyAutoDetectResult {
        detected: false,
        proxy: None,
        test: None,
        message: if last_error.is_empty() {
            "未识别到本机代理配置。".into()
        } else {
            format!("未识别到本机代理配置。最后一次识别结果：{last_error}")
        },
    })
}

#[tauri::command]
fn get_runtime_status() -> Result<RuntimeStatus, String> {
    collect_runtime_status()
}

#[tauri::command]
fn start_runtime_prepare(app: tauri::AppHandle) -> Result<ActionResult, String> {
    let status = collect_runtime_status()?;
    append_launcher_log(
        "INFO",
        format!(
            "runtime prepare requested {}",
            runtime_status_log_summary(&status)
        ),
    );
    if !runtime_prepare_requires_download(&status) {
        set_process_runtime_envs_from_status(&status);
        append_launcher_log(
            "INFO",
            "runtime prepare skipped because all runtimes are ready",
        );
        return Ok(ActionResult {
            ok: true,
            message: "检测到可用 Python / Java 运行环境，直接进入 Launcher。".into(),
            requires_download: Some(false),
        });
    }
    if RUNTIME_PREPARE_RUNNING.load(Ordering::Acquire) {
        append_launcher_log(
            "INFO",
            "runtime prepare request ignored because another prepare task is running",
        );
        return Ok(ActionResult {
            ok: true,
            message: "Python / Java 运行环境正在准备中...".into(),
            requires_download: Some(true),
        });
    }
    let guard = RuntimePrepareGuard::try_acquire()?;
    let app_for_task = app.clone();
    append_launcher_log("INFO", "runtime prepare worker starting");
    thread::spawn(move || {
        let _guard = guard;
        let emitter = RuntimeProgressEmitter::new(app_for_task.clone());
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prepare_private_runtimes(&app_for_task, Some(&emitter))
        }))
        .map_err(|payload| {
            format!(
                "运行环境准备线程异常：{}",
                panic_payload_to_string(payload.as_ref())
            )
        })
        .and_then(|result| result);
        match result {
            Ok(()) => {
                set_process_runtime_envs();
                append_launcher_log("INFO", "runtime prepare completed successfully");
                emitter.emit(
                    100.0,
                    100,
                    100,
                    "Python / Java 运行环境已准备完成。".into(),
                    Some("success"),
                );
            }
            Err(error) => {
                append_launcher_log("WARN", format!("runtime prepare failed: {error}"));
                emitter.emit(
                    100.0,
                    0,
                    0,
                    format!("Python / Java 运行环境准备失败：{error}"),
                    Some("failed"),
                );
            }
        }
    });
    Ok(ActionResult {
        ok: true,
        message: "正在准备缺失的 Python / Java 运行环境...".into(),
        requires_download: Some(true),
    })
}

#[tauri::command]
fn export_launcher_logs() -> Result<ActionResult, String> {
    let Some(folder) = rfd::FileDialog::new()
        .set_title("导出 BiblioSmith Launcher LOG")
        .pick_folder()
    else {
        return Ok(ActionResult {
            ok: false,
            message: "已取消导出 LOG。".into(),
            requires_download: None,
        });
    };
    let log_file = launcher_log_path()?;
    let log_dir = log_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| log_file.clone());
    let context = current_diagnostic_context()?;
    let export_dir = export_diagnostic_logs_to_dir(&folder, &log_dir, &context)?;
    append_launcher_log(
        "INFO",
        format!("diagnostic logs exported to {}", display_path(&export_dir)),
    );
    Ok(ActionResult {
        ok: true,
        message: format!("已导出 LOG：{}", display_path(&export_dir)),
        requires_download: None,
    })
}

#[tauri::command]
fn record_frontend_activity(level: String, message: String) -> Result<(), String> {
    let normalized_level = match level.to_ascii_lowercase().as_str() {
        "error" => "UI-ERROR",
        "warning" => "UI-WARN",
        "success" => "UI-SUCCESS",
        _ => "UI-INFO",
    };
    append_launcher_log(normalized_level, message);
    Ok(())
}

#[tauri::command]
fn minimize_main_window(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "无法定位 BiblioSmith Launcher 主窗口。".to_string())?;
    window.minimize().map_err(|err| err.to_string())
}

#[tauri::command]
fn toggle_main_window_maximized(app: tauri::AppHandle) -> Result<bool, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "无法定位 BiblioSmith Launcher 主窗口。".to_string())?;
    let is_maximized = window.is_maximized().map_err(|err| err.to_string())?;
    if is_maximized {
        window.unmaximize().map_err(|err| err.to_string())?;
        Ok(false)
    } else {
        window.maximize().map_err(|err| err.to_string())?;
        Ok(true)
    }
}

#[tauri::command]
fn close_main_window_to_tray(app: tauri::AppHandle) -> Result<(), String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "无法定位 BiblioSmith Launcher 主窗口。".to_string())?;
    window.hide().map_err(|err| err.to_string())
}

fn format_kb(bytes: f64) -> String {
    format!("{:.1} KB", (bytes / 1024.0).max(0.0))
}

#[cfg(target_os = "windows")]
const BIBLIOSMITH_ARCHIVE_ZIP_ENV: &str = "BIBLIOSMITH_ARCHIVE_ZIP";
#[cfg(target_os = "windows")]
const BIBLIOSMITH_ARCHIVE_DEST_ENV: &str = "BIBLIOSMITH_ARCHIVE_DEST";

#[cfg(target_os = "windows")]
fn windows_expand_archive_command_script() -> &'static str {
    "$ErrorActionPreference = 'Stop'; Expand-Archive -LiteralPath $env:BIBLIOSMITH_ARCHIVE_ZIP -DestinationPath $env:BIBLIOSMITH_ARCHIVE_DEST -Force"
}

#[cfg(target_os = "windows")]
fn extract_zip_archive(archive_file: &Path, destination: &Path) -> Result<(), String> {
    extract_zip_archive_with_windows_tools(
        archive_file,
        destination,
        &windows_powershell_candidates(),
        Path::new("tar"),
    )
}

#[cfg(target_os = "windows")]
fn windows_powershell_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for name in ["SystemRoot", "WINDIR"] {
        if let Some(root) = env::var_os(name) {
            candidates.push(
                PathBuf::from(root)
                    .join("System32")
                    .join("WindowsPowerShell")
                    .join("v1.0")
                    .join("powershell.exe"),
            );
        }
    }
    candidates.push(PathBuf::from("powershell"));
    candidates.push(PathBuf::from("pwsh"));
    let mut unique = Vec::new();
    for candidate in candidates {
        if !unique.iter().any(|item: &PathBuf| item == &candidate) {
            unique.push(candidate);
        }
    }
    unique
}

#[cfg(target_os = "windows")]
fn extract_zip_archive_with_windows_tools(
    archive_file: &Path,
    destination: &Path,
    powershell_candidates: &[PathBuf],
    tar_program: &Path,
) -> Result<(), String> {
    let mut powershell_errors = Vec::new();
    for powershell in powershell_candidates {
        match extract_zip_archive_with_powershell(powershell, archive_file, destination) {
            Ok(()) => return Ok(()),
            Err(error) => powershell_errors.push(error),
        }
    }
    match extract_zip_archive_with_tar_program(tar_program, archive_file, destination) {
        Ok(()) => Ok(()),
        Err(tar_error) => Err(format!(
            "解压 ZIP archive 失败：PowerShell: {}; tar: {tar_error}",
            powershell_errors.join(" | ")
        )),
    }
}

#[cfg(target_os = "windows")]
fn extract_zip_archive_with_powershell(
    powershell: &Path,
    archive_file: &Path,
    destination: &Path,
) -> Result<(), String> {
    let mut command = Command::new(powershell);
    command
        .arg("-NoProfile")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-Command")
        .arg(windows_expand_archive_command_script())
        .env(BIBLIOSMITH_ARCHIVE_ZIP_ENV, archive_file.as_os_str())
        .env(BIBLIOSMITH_ARCHIVE_DEST_ENV, destination.as_os_str());
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .map_err(|err| format!("{}: 无法启动：{err}", display_path(powershell)))?;
    if output.status.success() {
        return Ok(());
    }
    let powershell_error = String::from_utf8_lossy(&output.stderr).trim().to_string();
    Err(format!("{}: {powershell_error}", display_path(powershell)))
}

#[cfg(target_os = "windows")]
fn extract_zip_archive_with_tar_program(
    tar_program: &Path,
    archive_file: &Path,
    destination: &Path,
) -> Result<(), String> {
    let mut command = Command::new(tar_program);
    command
        .arg("-xf")
        .arg(archive_file)
        .arg("-C")
        .arg(destination);
    command.creation_flags(0x08000000);
    let output = command
        .output()
        .map_err(|err| format!("无法启动 tar 解压 ZIP archive：{err}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "tar 解压 ZIP archive 失败：{}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

#[cfg(target_os = "macos")]
fn extract_zip_archive(archive_file: &Path, destination: &Path) -> Result<(), String> {
    let output = Command::new("ditto")
        .arg("-x")
        .arg("-k")
        .arg(display_path(archive_file))
        .arg(display_path(destination))
        .output()
        .map_err(|err| format!("无法启动 ditto 解压 BiblioSmith archive：{err}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "解压 BiblioSmith archive 失败：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

#[cfg(all(unix, not(target_os = "macos")))]
fn extract_zip_archive(archive_file: &Path, destination: &Path) -> Result<(), String> {
    let output = Command::new("unzip")
        .arg("-q")
        .arg(display_path(archive_file))
        .arg("-d")
        .arg(display_path(destination))
        .output()
        .map_err(|err| format!("无法启动 unzip 解压 BiblioSmith archive：{err}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "解压 BiblioSmith archive 失败：{}",
        String::from_utf8_lossy(&output.stderr).trim()
    ))
}

fn flatten_extracted_archive_root(destination: &Path) -> Result<(), String> {
    let entries = fs::read_dir(destination)
        .map_err(|err| err.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|err| err.to_string())?;
    if entries.len() != 1 || !entries[0].path().is_dir() {
        return Ok(());
    }
    let root = entries[0].path();
    for entry in fs::read_dir(&root).map_err(|err| err.to_string())? {
        let entry = entry.map_err(|err| err.to_string())?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        fs::rename(&from, &to).map_err(|err| {
            format!(
                "无法整理 BiblioSmith archive 解压目录：{} -> {}：{err}",
                display_path(&from),
                display_path(&to)
            )
        })?;
    }
    fs::remove_dir_all(root).map_err(|err| err.to_string())
}

#[cfg(test)]
fn safe_archive_entry_relative_path(name: &str) -> Result<PathBuf, String> {
    let normalized = name.replace('\\', "/");
    let mut parts = normalized.split('/').filter(|part| !part.is_empty());
    let _github_root = parts
        .next()
        .ok_or_else(|| format!("BiblioSmith archive 条目路径无效：{name}"))?;
    let mut relative = PathBuf::new();
    let mut has_relative = false;
    for part in parts {
        if part == "." || part == ".." || part.contains(':') {
            return Err(format!("BiblioSmith archive 条目路径不安全：{name}"));
        }
        relative.push(part);
        has_relative = true;
    }
    if !has_relative {
        return Err(format!(
            "BiblioSmith archive 条目路径缺少可解压的相对路径：{name}"
        ));
    }
    Ok(relative)
}

fn runtime_packages() -> [RuntimePackage; 2] {
    [
        RuntimePackage {
            kind: RuntimeKind::Python,
            version: PYTHON_RUNTIME_VERSION,
            install_dir_name: PYTHON_RUNTIME_DIR_NAME,
            archive_name: PYTHON_RUNTIME_ARCHIVE,
            sha256: PYTHON_RUNTIME_SHA256,
            size_bytes: PYTHON_RUNTIME_SIZE_BYTES,
            urls: PYTHON_RUNTIME_URLS,
        },
        RuntimePackage {
            kind: RuntimeKind::Java,
            version: JAVA_RUNTIME_VERSION,
            install_dir_name: JAVA_RUNTIME_DIR_NAME,
            archive_name: JAVA_RUNTIME_ARCHIVE,
            sha256: JAVA_RUNTIME_SHA256,
            size_bytes: JAVA_RUNTIME_SIZE_BYTES,
            urls: JAVA_RUNTIME_URLS,
        },
    ]
}

fn collect_runtime_status() -> Result<RuntimeStatus, String> {
    let root = runtime_root()?;
    #[cfg(target_os = "windows")]
    let python = collect_runtime_tool_status(runtime_packages()[0], &root);
    #[cfg(not(target_os = "windows"))]
    let python = collect_bundled_uv_python_status();
    let java = collect_runtime_tool_status(runtime_packages()[1], &root);
    let private_ready = python.private_ready && java.private_ready;
    let status = RuntimeStatus {
        ready: python.ready && java.ready,
        private_ready,
        running: RUNTIME_PREPARE_RUNNING.load(Ordering::Acquire),
        runtime_root: display_path(&root),
        python,
        java,
    };
    if status.ready {
        set_process_runtime_envs_from_status(&status);
    }
    Ok(status)
}

#[cfg(not(target_os = "windows"))]
fn collect_bundled_uv_python_status() -> RuntimeToolStatus {
    let bundled = app_paths::current()
        .ok()
        .map(|paths| paths.runtime_bin_root().join("uv"));
    let development = cfg!(debug_assertions)
        .then(|| command_first_stdout_path("which", &["uv"]))
        .flatten();
    let path = bundled
        .filter(|path| path.is_file())
        .or(development)
        .map(|path| display_path(&path));
    let ready = path.is_some();
    RuntimeToolStatus {
        ready,
        private_ready: ready,
        version: "managed by bundled uv".into(),
        source: ready.then(|| "bundled_uv".into()),
        path,
        message: if ready {
            "Python 由 BiblioSmith 随包 uv 管理。".into()
        } else {
            "BiblioSmith 随包 uv 缺失，无法管理 Python。".into()
        },
    }
}

fn collect_runtime_tool_status(package: RuntimePackage, root: &Path) -> RuntimeToolStatus {
    let private_path = runtime_private_executable_from_root(root, package);
    let private_ready = private_path.as_ref().is_some_and(|path| path.is_file());
    let env_path = if private_ready {
        None
    } else {
        explicit_runtime_env_executable(package.kind)
    };
    let system_path = if private_ready || env_path.is_some() {
        None
    } else {
        system_runtime_executable(package.kind)
    };
    let source = if private_ready {
        Some("private".into())
    } else if env_path.is_some() {
        Some("env".into())
    } else if system_path.is_some() {
        Some("system".into())
    } else {
        None
    };
    let resolved_path = private_path
        .filter(|path| path.is_file())
        .or(env_path)
        .or(system_path);
    let ready = resolved_path.is_some();
    let path = resolved_path.as_ref().map(|path| display_path(path));
    let message = if private_ready {
        format!("{} 私有运行时已准备完成。", package.kind.label())
    } else if ready {
        format!("{} 已检测到可用的本机运行时。", package.kind.label())
    } else {
        format!(
            "{} 未检测到可用运行时，需要稍后安装。",
            package.kind.label()
        )
    };
    RuntimeToolStatus {
        ready,
        private_ready,
        version: package.version.into(),
        source,
        path,
        message,
    }
}

fn runtime_prepare_requires_download(status: &RuntimeStatus) -> bool {
    !status.ready
}

fn runtime_status_log_summary(status: &RuntimeStatus) -> String {
    format!(
        "ready={} private_ready={} running={} root={} python=[{}] java=[{}]",
        status.ready,
        status.private_ready,
        status.running,
        status.runtime_root,
        runtime_tool_log_summary(&status.python),
        runtime_tool_log_summary(&status.java)
    )
}

fn runtime_tool_log_summary(status: &RuntimeToolStatus) -> String {
    format!(
        "ready={} private_ready={} source={} path={} version={}",
        status.ready,
        status.private_ready,
        status.source.as_deref().unwrap_or("-"),
        status.path.as_deref().unwrap_or("-"),
        status.version
    )
}

fn runtime_root() -> Result<PathBuf, String> {
    Ok(app_paths::current()?.support_root().join("runtimes"))
}

fn runtime_install_dir_from_root(root: &Path, package: RuntimePackage) -> PathBuf {
    root.join(package.kind.dir_name())
        .join(package.install_dir_name)
}

fn runtime_downloads_dir_for(paths: &app_paths::AppPaths) -> PathBuf {
    paths.cache_root().join("runtimes").join("downloads")
}

fn runtime_private_executable_from_root(root: &Path, package: RuntimePackage) -> Option<PathBuf> {
    let install_dir = runtime_install_dir_from_root(root, package);
    match package.kind {
        RuntimeKind::Python => {
            #[cfg(target_os = "windows")]
            let candidate = install_dir.join("python.exe");
            #[cfg(not(target_os = "windows"))]
            let candidate = install_dir.join("bin").join("python3");
            candidate.is_file().then_some(candidate)
        }
        RuntimeKind::Java => find_runtime_executable(&install_dir, java_executable_name()),
    }
}

fn java_executable_name() -> &'static str {
    #[cfg(target_os = "windows")]
    {
        "java.exe"
    }
    #[cfg(not(target_os = "windows"))]
    {
        "java"
    }
}

fn runtime_resolved_executable(package: RuntimePackage) -> Option<PathBuf> {
    let root = runtime_root().ok();
    if let Some(root) = root.as_deref() {
        if let Some(path) = runtime_private_executable_from_root(root, package) {
            return Some(path);
        }
    }
    explicit_runtime_env_executable(package.kind)
        .or_else(|| system_runtime_executable(package.kind))
}

/// The Java the launcher manages: private runtime, then `BIBLIOSMITH_JAVA`, then
/// the system. Shared with the Book Pipeline runner so its EPUBCheck call
/// resolves Java exactly the way `run_epubcheck.js` already does.
pub(crate) fn managed_java_executable() -> Option<PathBuf> {
    managed_runtime_executable(RuntimeKind::Java)
}

/// The Python counterpart, for the bilingual builder the runner spawns directly.
/// `run_python.cjs` already honours `BIBLIOSMITH_PYTHON`; the Rust side has to
/// agree, or "运行时准备" reports green while the stage runs a different
/// interpreter.
pub(crate) fn managed_python_executable() -> Option<PathBuf> {
    managed_runtime_executable(RuntimeKind::Python)
}

fn managed_runtime_executable(kind: RuntimeKind) -> Option<PathBuf> {
    runtime_packages()
        .into_iter()
        .find(|package| package.kind == kind)
        .and_then(runtime_resolved_executable)
}

fn explicit_runtime_env_executable(kind: RuntimeKind) -> Option<PathBuf> {
    let path = env::var_os(kind.env_name()).map(PathBuf::from)?;
    runtime_executable_is_usable(kind, &path).then_some(path)
}

fn system_runtime_executable(kind: RuntimeKind) -> Option<PathBuf> {
    match kind {
        RuntimeKind::Python => system_python_executable(),
        RuntimeKind::Java => system_java_executable(),
    }
}

fn system_python_executable() -> Option<PathBuf> {
    let probes: &[(&str, &[&str])] = &[
        ("python", &["-c", "import sys; print(sys.executable)"]),
        ("py", &["-3", "-c", "import sys; print(sys.executable)"]),
        ("python3", &["-c", "import sys; print(sys.executable)"]),
    ];
    probes.iter().find_map(|(program, args)| {
        command_first_stdout_path(program, args)
            .filter(|path| runtime_executable_is_usable(RuntimeKind::Python, path))
    })
}

fn system_java_executable() -> Option<PathBuf> {
    if let Some(path) = env::var_os("JAVA_HOME")
        .map(PathBuf::from)
        .and_then(|java_home| java_home_executable_from_value(&java_home))
        .filter(|path| runtime_executable_is_usable(RuntimeKind::Java, path))
    {
        return Some(path);
    }

    if let Some(path) = java_path_from_path_lookup()
        .into_iter()
        .find(|path| runtime_executable_is_usable(RuntimeKind::Java, path))
    {
        return Some(path);
    }

    common_java_install_roots()
        .into_iter()
        .filter(|root| root.is_dir())
        .find_map(|root| {
            find_runtime_executable_limited(&root, java_executable_name(), 5, 120)
                .filter(|path| runtime_executable_is_usable(RuntimeKind::Java, path))
        })
}

fn java_home_executable_from_value(java_home: &Path) -> Option<PathBuf> {
    let candidate = java_home.join("bin").join(java_executable_name());
    candidate.is_file().then_some(candidate)
}

pub(crate) fn command_output_with_timeout(
    command: &mut Command,
    timeout: Duration,
) -> io::Result<Output> {
    command.stdout(Stdio::piped());
    // The Book Pipeline reads stderr for its failure diagnosis, and
    // `wait_with_output` only captures what was piped.
    command.stderr(Stdio::piped());
    let mut child = command.spawn()?;
    // Drained on their own threads, not after the wait loop: a pipe holds ~64 KB
    // before `write` blocks, and a child parked in that write never exits. Read
    // only once it has exited and the loop below waits for something that cannot
    // happen — the child waits for us, we wait for the child, and the timeout
    // "expires" on a command that had already finished its work. The engine
    // prints its whole run report to stdout in one go, so this is reachable by a
    // book with enough chapters rather than by a misbehaving child.
    let mut stdout_pipe = child.stdout.take();
    let mut stderr_pipe = child.stderr.take();
    let stdout_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stdout_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let stderr_reader = thread::spawn(move || {
        let mut buffer = Vec::new();
        if let Some(pipe) = stderr_pipe.as_mut() {
            let _ = pipe.read_to_end(&mut buffer);
        }
        buffer
    });
    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Output {
                status,
                stdout: stdout_reader.join().unwrap_or_default(),
                stderr: stderr_reader.join().unwrap_or_default(),
            });
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            // Deliberately not joined. Killing the child does not kill any
            // grandchild it left holding the write end, so a join here could
            // block forever — turning the bound we just enforced back into the
            // unbounded wait it exists to prevent. The output is discarded on
            // this path anyway.
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command timed out after {}s", timeout.as_secs()),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn command_status_with_timeout(command: &mut Command, timeout: Duration) -> io::Result<ExitStatus> {
    let mut child = command.spawn()?;
    let started_at = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if started_at.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("command timed out after {}s", timeout.as_secs()),
            ));
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn command_first_stdout_path(program: &str, args: &[&str]) -> Option<PathBuf> {
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    let output = command_output_with_timeout(
        &mut command,
        Duration::from_secs(RUNTIME_PROBE_TIMEOUT_SECONDS),
    )
    .map_err(|err| {
        append_launcher_log(
            "WARN",
            format!("runtime probe command failed program={program} error={err}"),
        );
        err
    })
    .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .find(|path| path.is_file())
}

fn java_path_from_path_lookup() -> Vec<PathBuf> {
    #[cfg(target_os = "windows")]
    let lookup = ("where", vec!["java"]);
    #[cfg(not(target_os = "windows"))]
    let lookup = ("which", vec!["java"]);

    let mut command = Command::new(lookup.0);
    command
        .args(lookup.1)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    let Ok(output) = command_output_with_timeout(
        &mut command,
        Duration::from_secs(RUNTIME_PROBE_TIMEOUT_SECONDS),
    ) else {
        append_launcher_log("WARN", "Java PATH lookup command failed or timed out");
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|path| path.is_file())
        .collect()
}

#[cfg(target_os = "windows")]
fn common_java_install_roots() -> Vec<PathBuf> {
    let mut roots = vec![
        PathBuf::from(r"C:\Program Files\Java"),
        PathBuf::from(r"C:\Program Files\Eclipse Adoptium"),
        PathBuf::from(r"C:\Program Files\Zulu"),
        PathBuf::from(r"C:\Program Files\Amazon Corretto"),
    ];
    if let Ok(entries) = fs::read_dir(r"C:\Program Files\Microsoft") {
        roots.extend(entries.flatten().map(|entry| entry.path()).filter(|path| {
            path.file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.to_ascii_lowercase().contains("jdk"))
        }));
    }
    roots
}

#[cfg(target_os = "macos")]
fn common_java_install_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/Library/Java/JavaVirtualMachines")]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn common_java_install_roots() -> Vec<PathBuf> {
    vec![PathBuf::from("/usr/lib/jvm"), PathBuf::from("/usr/java")]
}

fn runtime_executable_is_usable(kind: RuntimeKind, path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    let mut command = Command::new(path);
    match kind {
        RuntimeKind::Python => {
            command.arg("--version");
        }
        RuntimeKind::Java => {
            command.arg("-version");
        }
    }
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(target_os = "windows")]
    command.creation_flags(0x08000000);
    match command_status_with_timeout(
        &mut command,
        Duration::from_secs(RUNTIME_PROBE_TIMEOUT_SECONDS),
    ) {
        Ok(status) => status.success(),
        Err(error) => {
            append_launcher_log(
                "WARN",
                format!(
                    "runtime executable probe failed kind={} path={} error={error}",
                    kind.label(),
                    display_path(path)
                ),
            );
            false
        }
    }
}

fn find_runtime_executable(root: &Path, executable_name: &str) -> Option<PathBuf> {
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(executable_name))
        {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = find_runtime_executable(&path, executable_name) {
                return Some(found);
            }
        }
    }
    None
}

fn find_runtime_executable_limited(
    root: &Path,
    executable_name: &str,
    depth: usize,
    remaining: usize,
) -> Option<PathBuf> {
    if depth == 0 || remaining == 0 {
        return None;
    }
    let entries = fs::read_dir(root).ok()?;
    let mut remaining = remaining;
    for entry in entries.flatten() {
        if remaining == 0 {
            return None;
        }
        remaining -= 1;
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .and_then(|value| value.to_str())
                .is_some_and(|name| name.eq_ignore_ascii_case(executable_name))
        {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) =
                find_runtime_executable_limited(&path, executable_name, depth - 1, remaining)
            {
                return Some(found);
            }
        }
    }
    None
}

fn prepare_private_runtimes(
    _app: &tauri::AppHandle,
    progress: Option<&RuntimeProgressEmitter>,
) -> Result<(), String> {
    let paths = app_paths::current()?;
    let root = runtime_root()?;
    let downloads_dir = runtime_downloads_dir_for(&paths);
    fs::create_dir_all(&root).map_err(|err| err.to_string())?;
    fs::create_dir_all(&downloads_dir).map_err(|err| err.to_string())?;
    append_launcher_log(
        "INFO",
        format!("runtime prepare private root={}", display_path(&root)),
    );
    let packages = runtime_packages();
    for (index, package) in packages.iter().enumerate() {
        if package.kind == RuntimeKind::Python && !cfg!(target_os = "windows") {
            continue;
        }
        if let Some(executable) = runtime_resolved_executable(*package) {
            append_launcher_log(
                "INFO",
                format!(
                    "runtime package skipped kind={} executable={}",
                    package.kind.label(),
                    display_path(&executable)
                ),
            );
            if let Some(emitter) = progress {
                let percent = if index == 0 { 45.0 } else { 90.0 };
                emitter.emit(
                    percent,
                    package.size_bytes,
                    package.size_bytes,
                    format!("{} 已检测到可用运行时，跳过下载。", package.kind.label()),
                    Some("downloading"),
                );
            }
            continue;
        }
        prepare_runtime_package(&root, &downloads_dir, *package, progress, index)?;
    }
    if collect_runtime_status()?.ready {
        append_launcher_log("INFO", "runtime prepare private runtimes verified ready");
        Ok(())
    } else {
        append_launcher_log(
            "WARN",
            "runtime prepare finished but runtime status is not ready",
        );
        Err("Python / Java 运行环境未全部准备完成。".into())
    }
}

fn prepare_runtime_package(
    root: &Path,
    downloads_dir: &Path,
    package: RuntimePackage,
    progress: Option<&RuntimeProgressEmitter>,
    index: usize,
) -> Result<(), String> {
    let start = if index == 0 { 2.0 } else { 47.0 };
    let download_end = if index == 0 { 38.0 } else { 83.0 };
    let extract_end = if index == 0 { 45.0 } else { 92.0 };
    fs::create_dir_all(downloads_dir).map_err(|err| err.to_string())?;
    let archive = downloads_dir.join(package.archive_name);
    let mut last_error = String::new();
    append_launcher_log(
        "INFO",
        format!(
            "runtime package prepare kind={} version={} archive={}",
            package.kind.label(),
            package.version,
            display_path(&archive)
        ),
    );

    if archive.is_file() && runtime_archive_sha256_matches(&archive, package.sha256) {
        append_launcher_log(
            "INFO",
            format!(
                "runtime archive already downloaded kind={} path={}",
                package.kind.label(),
                display_path(&archive)
            ),
        );
    } else {
        let _ = fs::remove_file(&archive);
        for url in package.urls {
            if let Some(emitter) = progress {
                emitter.emit(
                    start,
                    0,
                    package.size_bytes,
                    format!("正在下载 {} 运行环境...", package.kind.label()),
                    Some("downloading"),
                );
            }
            match download_runtime_archive_from_url(
                package,
                url,
                &archive,
                progress,
                start,
                download_end,
            )
            .and_then(|_| verify_runtime_archive(&archive, package))
            {
                Ok(()) => {
                    append_launcher_log(
                        "INFO",
                        format!(
                            "runtime download verified kind={} url={}",
                            package.kind.label(),
                            url
                        ),
                    );
                    last_error.clear();
                    break;
                }
                Err(error) => {
                    last_error = format!("{url}: {error}");
                    append_launcher_log(
                        "WARN",
                        format!(
                            "runtime download failed kind={} url={} error={error}",
                            package.kind.label(),
                            url
                        ),
                    );
                    let _ = fs::remove_file(&archive);
                }
            }
        }
        if !last_error.is_empty() {
            return Err(format!(
                "{} 运行环境下载失败，已尝试所有下载源。最后错误：{}",
                package.kind.label(),
                last_error
            ));
        }
    }

    if let Some(emitter) = progress {
        emitter.emit(
            download_end,
            package.size_bytes,
            package.size_bytes,
            format!("正在校验并解压 {} 运行环境...", package.kind.label()),
            Some("downloading"),
        );
    }
    install_runtime_archive(root, package, &archive)?;
    append_launcher_log(
        "INFO",
        format!(
            "runtime package installed kind={} install_dir={}",
            package.kind.label(),
            display_path(&root.join(package.install_dir_name))
        ),
    );
    if let Some(emitter) = progress {
        emitter.emit(
            extract_end,
            package.size_bytes,
            package.size_bytes,
            format!("{} 运行环境已准备完成。", package.kind.label()),
            Some("downloading"),
        );
    }
    Ok(())
}

fn download_runtime_archive_from_url(
    package: RuntimePackage,
    url: &str,
    destination: &Path,
    progress: Option<&RuntimeProgressEmitter>,
    start_percent: f64,
    end_percent: f64,
) -> Result<(), String> {
    append_launcher_log(
        "INFO",
        format!(
            "downloading runtime kind={} url={url}",
            package.kind.label()
        ),
    );
    let client = runtime_http_blocking_client()?;
    let mut response = client
        .get(url)
        .header("User-Agent", "BiblioSmith-Launcher")
        .send()
        .map_err(|err| format!("下载失败：{err}"))?
        .error_for_status()
        .map_err(|err| format!("下载失败：{err}"))?;
    let total = response.content_length().unwrap_or(package.size_bytes);
    let mut file = fs::File::create(destination).map_err(|err| err.to_string())?;
    let mut buffer = [0_u8; 64 * 1024];
    let mut downloaded = 0_u64;
    let started_at = Instant::now();
    let mut last_emit_at = Instant::now() - Duration::from_secs(2);
    loop {
        let read = response
            .read(&mut buffer)
            .map_err(|err| format!("读取下载数据失败：{err}"))?;
        if read == 0 {
            break;
        }
        file.write_all(&buffer[..read])
            .map_err(|err| format!("写入下载文件失败：{err}"))?;
        downloaded += read as u64;
        if let Some(emitter) = progress {
            if last_emit_at.elapsed() >= Duration::from_millis(300) {
                let span = end_percent - start_percent;
                let percent = if total > 0 {
                    start_percent + (downloaded as f64 / total as f64) * span
                } else {
                    start_percent
                };
                emitter.emit(
                    percent,
                    downloaded,
                    total,
                    format!(
                        "正在下载 {} 运行环境... {}",
                        package.kind.label(),
                        runtime_download_detail(downloaded, total, started_at.elapsed())
                    ),
                    Some("downloading"),
                );
                last_emit_at = Instant::now();
            }
        }
    }
    file.flush().map_err(|err| err.to_string())?;
    if total > 0 && downloaded < total {
        return Err(format!("下载未完成：{downloaded} / {total} bytes"));
    }
    if let Some(emitter) = progress {
        emitter.emit(
            end_percent,
            downloaded,
            total,
            format!(
                "{} 运行环境下载完成。{}",
                package.kind.label(),
                runtime_download_detail(downloaded, total, started_at.elapsed())
            ),
            Some("downloading"),
        );
    }
    Ok(())
}

fn verify_runtime_archive(archive: &Path, package: RuntimePackage) -> Result<(), String> {
    let actual = sha256_file(archive)?;
    if actual.eq_ignore_ascii_case(package.sha256) {
        return Ok(());
    }
    Err(format!(
        "{} SHA256 校验失败：期望 {}，实际 {}",
        package.kind.label(),
        package.sha256,
        actual
    ))
}

fn runtime_archive_sha256_matches(archive: &Path, expected: &str) -> bool {
    sha256_file(archive).is_ok_and(|actual| actual.eq_ignore_ascii_case(expected))
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|err| err.to_string())?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(format!("{:X}", hasher.finalize()))
}

fn install_runtime_archive(
    root: &Path,
    package: RuntimePackage,
    archive: &Path,
) -> Result<(), String> {
    let kind_root = root.join(package.kind.dir_name());
    fs::create_dir_all(&kind_root).map_err(|err| err.to_string())?;
    let final_dir = runtime_install_dir_from_root(root, package);
    let temp_dir = kind_root.join(format!("{}.tmp", package.install_dir_name));
    if temp_dir.exists() {
        fs::remove_dir_all(&temp_dir).map_err(|err| err.to_string())?;
    }
    fs::create_dir_all(&temp_dir).map_err(|err| err.to_string())?;
    extract_zip_archive(archive, &temp_dir)?;
    flatten_extracted_archive_root(&temp_dir)?;
    if final_dir.exists() {
        fs::remove_dir_all(&final_dir).map_err(|err| err.to_string())?;
    }
    fs::rename(&temp_dir, &final_dir).map_err(|err| {
        format!(
            "无法安装 {} 运行环境：{} -> {}：{err}",
            package.kind.label(),
            display_path(&temp_dir),
            display_path(&final_dir)
        )
    })?;
    if runtime_private_executable_from_root(root, package).is_some_and(|path| path.is_file()) {
        Ok(())
    } else {
        Err(format!("{} 解压后未找到可执行文件。", package.kind.label()))
    }
}

fn runtime_download_detail(downloaded: u64, total: u64, elapsed: Duration) -> String {
    let percent = download_percent(downloaded, total);
    let seconds = elapsed.as_secs_f64().max(0.001);
    let speed = (downloaded as f64 / seconds).round() as u64;
    if total > 0 {
        format!(
            "{} ({} / {}, {}/s)",
            format_runtime_percent(percent),
            format_kb(downloaded as f64),
            format_kb(total as f64),
            format_kb(speed as f64)
        )
    } else {
        format!(
            "{} ({}, {}/s)",
            format_runtime_percent(percent),
            format_kb(downloaded as f64),
            format_kb(speed as f64)
        )
    }
}

fn format_runtime_percent(value: f64) -> String {
    format!("{:.2}%", clamp_progress_percent(value))
}

impl RuntimeProgressEmitter {
    fn new(app: tauri::AppHandle) -> Self {
        Self { app }
    }

    fn emit(
        &self,
        percent: f64,
        downloaded_bytes: u64,
        total_bytes: u64,
        message: String,
        state: Option<&str>,
    ) {
        let payload = DownloadProgress {
            percent: clamp_progress_percent(percent),
            downloaded_bytes,
            total_bytes,
            message: Some(message),
            state: state.map(|value| value.to_string()),
        };
        let _ = self.app.emit(RUNTIME_PROGRESS_EVENT, payload.clone());
        if let Some(window) = self.app.get_webview_window("main") {
            let _ = window.emit(RUNTIME_PROGRESS_EVENT, payload);
        }
    }
}

fn clamp_progress_percent(value: f64) -> f64 {
    if !value.is_finite() {
        return 0.0;
    }
    (value.clamp(0.0, 100.0) * 100.0).round() / 100.0
}

fn launcher_config_path() -> Result<PathBuf, String> {
    Ok(app_paths::current()?.support_root().join("config.json"))
}

#[cfg(test)]
fn launcher_config_path_from_base(base: &Path, development: bool) -> PathBuf {
    let launcher_dir = if development {
        "launcher-dev"
    } else {
        "launcher"
    };
    base.join("BiblioSmith")
        .join(launcher_dir)
        .join("config.json")
}

fn read_launcher_config() -> Option<LauncherConfig> {
    let path = launcher_config_path().ok()?;
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn write_launcher_config_file(config: &LauncherConfig) -> Result<(), String> {
    let path = launcher_config_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|err| err.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|err| err.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| "BiblioSmith 配置文件缺少父目录。".to_string())?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|err| err.to_string())?;
    temporary
        .write_all(text.as_bytes())
        .and_then(|_| temporary.as_file_mut().sync_all())
        .map_err(|err| err.to_string())?;
    temporary
        .persist(&path)
        .map_err(|error| error.error.to_string())?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|err| err.to_string())?;
    Ok(())
}

fn update_launcher_config(update: impl FnOnce(&mut LauncherConfig)) -> Result<(), String> {
    let _guard = LAUNCHER_CONFIG_LOCK
        .lock()
        .map_err(|_| "BiblioSmith 配置锁不可用。".to_string())?;
    let mut config = read_launcher_config().unwrap_or_default();
    update(&mut config);
    write_launcher_config_file(&config)
}

fn configured_workspace_root_from_config(config: Option<&LauncherConfig>) -> Option<PathBuf> {
    config
        .and_then(|config| config.workspace_root.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub(crate) fn configured_or_recommended_workspace_root() -> Result<PathBuf, String> {
    let config = read_launcher_config();
    Ok(configured_workspace_root_from_config(config.as_ref())
        .unwrap_or(app_paths::current()?.recommended_workspace_root()))
}

fn write_workspace_root(workspace_root: &Path) -> Result<(), String> {
    update_launcher_config(|config| {
        config.workspace_root = Some(display_path(workspace_root));
    })?;
    append_launcher_log(
        "INFO",
        format!("workspace_root={}", display_path(workspace_root)),
    );
    Ok(())
}

fn write_save_logs_config(save_logs: bool) -> Result<(), String> {
    update_launcher_config(|config| config.save_logs = Some(save_logs))
}

fn configured_proxy_settings() -> NetworkProxySettings {
    read_launcher_config()
        .and_then(|config| config.proxy)
        .unwrap_or_default()
}

fn write_proxy_config(proxy: NetworkProxySettings) -> Result<NetworkProxySettings, String> {
    validate_proxy_settings(&proxy)?;
    update_launcher_config(|config| config.proxy = Some(proxy.clone()))?;
    append_launcher_log(
        "INFO",
        format!(
            "network proxy updated enabled={} scheme={} host={} port={:?}",
            proxy.enabled, proxy.scheme, proxy.host, proxy.port
        ),
    );
    Ok(proxy)
}

fn validate_proxy_settings(proxy: &NetworkProxySettings) -> Result<(), String> {
    let scheme = normalized_proxy_scheme(&proxy.scheme)?;
    if proxy.enabled {
        if proxy.host.trim().is_empty() {
            return Err("代理 IP/主机不能为空。".into());
        }
        let port = proxy.port.ok_or_else(|| "代理端口不能为空。".to_string())?;
        if port == 0 {
            return Err("代理端口必须在 1-65535 之间。".into());
        }
    }
    if scheme.is_empty() {
        return Err("代理协议不能为空。".into());
    }
    Ok(())
}

fn normalized_proxy_scheme(value: &str) -> Result<String, String> {
    let scheme = value.trim().to_ascii_lowercase();
    match scheme.as_str() {
        "http" | "https" | "socks5" | "socks5h" => Ok(scheme),
        _ => Err("代理协议只支持 http、https、socks5、socks5h。".into()),
    }
}

fn configured_proxy_url() -> Result<Option<String>, String> {
    proxy_url_from_settings(&configured_proxy_settings())
}

fn configured_proxy_url_best_effort() -> Option<String> {
    configured_proxy_url().ok().flatten()
}

fn proxy_url_from_settings(proxy: &NetworkProxySettings) -> Result<Option<String>, String> {
    validate_proxy_settings(proxy)?;
    if !proxy.enabled {
        return Ok(None);
    }
    let scheme = normalized_proxy_scheme(&proxy.scheme)?;
    let host = proxy.host.trim();
    let port = proxy.port.ok_or_else(|| "代理端口不能为空。".to_string())?;
    Ok(Some(format!("{scheme}://{host}:{port}")))
}

fn proxy_settings_from_url(value: &str) -> Option<NetworkProxySettings> {
    let trimmed = value.trim().trim_matches('"').trim_matches('\'');
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("direct") {
        return None;
    }
    if trimmed.contains(';') {
        return trimmed
            .split(';')
            .filter_map(|part| {
                let value = part.split_once('=').map(|(_, value)| value).unwrap_or(part);
                proxy_settings_from_url(value)
            })
            .next();
    }
    let (scheme_hint, raw) = trimmed
        .split_once('=')
        .map(|(key, value)| (Some(key.trim().to_ascii_lowercase()), value.trim()))
        .unwrap_or((None, trimmed));
    let candidate = if raw.contains("://") {
        raw.to_string()
    } else {
        let scheme = match scheme_hint.as_deref() {
            Some("socks") | Some("socks5") => "socks5",
            Some("socks5h") => "socks5h",
            _ => "http",
        };
        format!("{scheme}://{raw}")
    };
    let url = reqwest::Url::parse(&candidate).ok()?;
    let scheme = normalized_proxy_scheme(url.scheme()).ok()?;
    let host = url.host_str()?.trim_matches(['[', ']']).to_string();
    if host.trim().is_empty() || host.contains(' ') {
        return None;
    }
    let port = url.port()?;
    Some(NetworkProxySettings {
        enabled: true,
        scheme,
        host,
        port: Some(port),
    })
}

fn proxy_detection_candidates_with_current(
    _current: &NetworkProxySettings,
) -> Vec<NetworkProxySettings> {
    let mut candidates = Vec::new();
    // Auto-detect must read the computer's proxy configuration, not the current
    // UI draft. The user can type arbitrary values; only "测试连接" should use them.
    for name in [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ] {
        if let Ok(value) = env::var(name) {
            if let Some(proxy) = proxy_settings_from_url(&value) {
                candidates.push(proxy);
            }
        }
    }
    candidates.extend(system_proxy_candidates());
    candidates.extend(static_proxy_detection_candidates());
    dedupe_proxy_candidates(candidates)
}

fn static_proxy_detection_candidates() -> Vec<NetworkProxySettings> {
    let mut candidates = Vec::new();
    for value in [
        "http://127.0.0.1:7890",
        "http://127.0.0.1:7897",
        "socks5h://127.0.0.1:10808",
        "socks5://127.0.0.1:10808",
        "http://127.0.0.1:10809",
        "http://127.0.0.1:10808",
        "socks5h://127.0.0.1:7891",
        "http://127.0.0.1:20171",
        "http://localhost:7890",
        "socks5h://localhost:10808",
        "socks5://localhost:10808",
        "http://localhost:10809",
    ] {
        if let Some(proxy) = proxy_settings_from_url(value) {
            candidates.push(proxy);
        }
    }
    dedupe_proxy_candidates(candidates)
}

fn proxy_candidate_port_open_quick(candidate: &NetworkProxySettings) -> bool {
    if !is_loopback_proxy_host(&candidate.host) {
        return true;
    }
    let Some(port) = candidate.port else {
        return false;
    };
    let host = candidate.host.trim().trim_matches(['[', ']']);
    let address = if host.contains(':') {
        format!("[{host}]:{port}")
    } else {
        format!("{host}:{port}")
    };
    let Ok(addresses) = address.to_socket_addrs() else {
        return false;
    };
    let timeout = Duration::from_millis(PROXY_PORT_PROBE_TIMEOUT_MS);
    addresses
        .into_iter()
        .any(|address| TcpStream::connect_timeout(&address, timeout).is_ok())
}

fn is_loopback_proxy_host(host: &str) -> bool {
    let host = host.trim().trim_matches(['[', ']']).to_ascii_lowercase();
    if host == "localhost" {
        return true;
    }
    host.parse::<IpAddr>()
        .map(|address| address.is_loopback())
        .unwrap_or(false)
}

fn proxy_candidate_label(candidate: &NetworkProxySettings, detail: &str) -> String {
    format!(
        "{}://{}:{} {detail}",
        candidate.scheme,
        candidate.host,
        candidate
            .port
            .map(|port| port.to_string())
            .unwrap_or_else(|| "?".into())
    )
}

fn dedupe_proxy_candidates(candidates: Vec<NetworkProxySettings>) -> Vec<NetworkProxySettings> {
    let mut unique = Vec::new();
    for candidate in candidates {
        let Ok(Some(url)) = proxy_url_from_settings(&candidate) else {
            continue;
        };
        let exists = unique
            .iter()
            .filter_map(|item| proxy_url_from_settings(item).ok().flatten())
            .any(|existing| existing.eq_ignore_ascii_case(&url));
        if !exists {
            unique.push(candidate);
        }
    }
    unique
}

#[cfg(target_os = "windows")]
fn system_proxy_candidates() -> Vec<NetworkProxySettings> {
    let mut candidates = Vec::new();
    if let Some(proxy_server) = windows_internet_proxy_server() {
        candidates.extend(parse_proxy_server_list(&proxy_server));
    }
    if let Some(proxy_server) = winhttp_proxy_server() {
        candidates.extend(parse_proxy_server_list(&proxy_server));
    }
    candidates
}

#[cfg(not(target_os = "windows"))]
fn system_proxy_candidates() -> Vec<NetworkProxySettings> {
    Vec::new()
}

#[cfg(target_os = "windows")]
fn windows_internet_proxy_server() -> Option<String> {
    let key = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    let enable_output = hidden_command_output("reg", &["query", key, "/v", "ProxyEnable"]).ok()?;
    let proxy_enabled = enable_output
        .lines()
        .any(|line| line.contains("ProxyEnable") && (line.contains("0x1") || line.ends_with(" 1")));
    if !proxy_enabled {
        return None;
    }
    let server_output = hidden_command_output("reg", &["query", key, "/v", "ProxyServer"]).ok()?;
    registry_value_tail(&server_output, "ProxyServer")
}

#[cfg(target_os = "windows")]
fn winhttp_proxy_server() -> Option<String> {
    let output = hidden_command_output("netsh", &["winhttp", "show", "proxy"]).ok()?;
    output
        .lines()
        .filter_map(|line| {
            line.split_once(':')
                .map(|(_, value)| value.trim().to_string())
        })
        .find(|value| {
            !value.is_empty()
                && !value.to_ascii_lowercase().contains("direct")
                && (value.contains(':') || value.contains('='))
        })
}

#[cfg(target_os = "windows")]
fn hidden_command_output(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    command.args(args);
    command.creation_flags(0x08000000);
    let output = command.output().map_err(|err| err.to_string())?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&output.stdout).to_string())
}

#[cfg(target_os = "windows")]
fn registry_value_tail(output: &str, name: &str) -> Option<String> {
    output.lines().find_map(|line| {
        if !line.contains(name) {
            return None;
        }
        line.split_whitespace()
            .last()
            .map(|value| value.to_string())
    })
}

// Only the Windows system_proxy_candidates reads the registry values this
// parses; the non-Windows build has no caller and no test.
#[cfg(target_os = "windows")]
fn parse_proxy_server_list(value: &str) -> Vec<NetworkProxySettings> {
    let mut proxies = Vec::new();
    for part in value.split(';') {
        let raw = part.trim();
        if raw.is_empty() {
            continue;
        }
        if let Some(proxy) = proxy_settings_from_url(raw) {
            proxies.push(proxy);
        }
    }
    if proxies.is_empty() {
        if let Some(proxy) = proxy_settings_from_url(value) {
            proxies.push(proxy);
        }
    }
    proxies
}

fn apply_reqwest_proxy(
    builder: reqwest::ClientBuilder,
    proxy_url: Option<&str>,
) -> Result<reqwest::ClientBuilder, String> {
    if let Some(proxy_url) = proxy_url {
        let proxy = reqwest::Proxy::all(proxy_url).map_err(|err| format!("代理配置无效：{err}"))?;
        Ok(builder.proxy(proxy))
    } else if let Some(proxy_url) = configured_proxy_url_best_effort() {
        let proxy =
            reqwest::Proxy::all(&proxy_url).map_err(|err| format!("代理配置无效：{err}"))?;
        Ok(builder.proxy(proxy))
    } else {
        Ok(builder)
    }
}

fn diagnostic_log_settings() -> Result<DiagnosticLogSettings, String> {
    let log_file = launcher_log_path()?;
    let log_dir = log_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| log_file.clone());
    Ok(DiagnosticLogSettings {
        save_logs: launcher_logging_enabled(),
        log_dir: display_path(&log_dir),
        log_file: display_path(&log_file),
        max_bytes: LAUNCHER_LOG_MAX_BYTES,
        backup_count: LAUNCHER_LOG_BACKUP_COUNT,
        max_total_bytes: LAUNCHER_LOG_MAX_BYTES * (LAUNCHER_LOG_BACKUP_COUNT as u64 + 1),
    })
}

fn diagnostic_context_for_export(
    launcher_version: &str,
    workspace_root: &str,
    workspace_status: app_paths::WorkspaceStatus,
    save_logs: bool,
    log_dir: &Path,
    log_max_bytes: u64,
    log_backup_count: usize,
) -> DiagnosticExportContext {
    DiagnosticExportContext {
        generated_at: Local::now().to_rfc3339(),
        launcher_version: launcher_version.to_string(),
        os: env::consts::OS.to_string(),
        arch: env::consts::ARCH.to_string(),
        workspace_root: workspace_root.to_string(),
        workspace_status,
        save_logs,
        log_dir: display_path(log_dir),
        log_max_bytes,
        log_backup_count,
        proxy_configured: is_proxy_configured(),
    }
}

fn current_diagnostic_context() -> Result<DiagnosticExportContext, String> {
    let workspace_root = configured_or_recommended_workspace_root()?;
    let workspace_status = app_paths::workspace_status(&workspace_root);
    let log_file = launcher_log_path()?;
    let log_dir = log_file
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| log_file.clone());
    Ok(diagnostic_context_for_export(
        &launcher_current_version(),
        &display_path(&workspace_root),
        workspace_status,
        launcher_logging_enabled(),
        &log_dir,
        LAUNCHER_LOG_MAX_BYTES,
        LAUNCHER_LOG_BACKUP_COUNT,
    ))
}

fn diagnostic_log_files(log_dir: &Path) -> Vec<PathBuf> {
    let current = log_dir.join("bibliosmith-launcher.log");
    let mut files = Vec::new();
    if current.is_file() {
        files.push(current.clone());
    }
    for index in 1..=LAUNCHER_LOG_LEGACY_EXPORT_BACKUP_SCAN_COUNT {
        let rotated = rotated_log_path(&current, index);
        if rotated.is_file() {
            files.push(rotated);
        }
    }
    files
}

fn export_diagnostic_logs_to_dir(
    export_parent: &Path,
    log_dir: &Path,
    context: &DiagnosticExportContext,
) -> Result<PathBuf, String> {
    fs::create_dir_all(export_parent).map_err(|err| err.to_string())?;
    let export_dir = export_parent.join(format!(
        "BiblioSmith-Launcher-Logs-{}",
        Local::now().format("%Y%m%d-%H%M%S")
    ));
    fs::create_dir_all(&export_dir).map_err(|err| err.to_string())?;
    for source in diagnostic_log_files(log_dir) {
        if let Some(file_name) = source.file_name() {
            fs::copy(&source, export_dir.join(file_name)).map_err(|err| err.to_string())?;
        }
    }
    let context_text = serde_json::to_string_pretty(context).map_err(|err| err.to_string())?;
    fs::write(export_dir.join("diagnostic-context.json"), context_text)
        .map_err(|err| err.to_string())?;
    Ok(export_dir)
}

fn set_process_runtime_envs() {
    for package in runtime_packages() {
        if package.kind == RuntimeKind::Python && !cfg!(target_os = "windows") {
            continue;
        }
        if let Some(path) = runtime_resolved_executable(package) {
            env::set_var(package.kind.env_name(), display_path(&path));
        }
    }
}

fn set_process_runtime_envs_from_status(status: &RuntimeStatus) {
    for (kind, tool) in [
        (RuntimeKind::Python, &status.python),
        (RuntimeKind::Java, &status.java),
    ] {
        if tool.ready && tool.source.as_deref() != Some("bundled_uv") {
            if let Some(path) = tool.path.as_deref() {
                env::set_var(kind.env_name(), path);
            }
        }
    }
}

fn display_path(path: &Path) -> String {
    let raw = path.display().to_string();
    if let Some(value) = raw.strip_prefix("\\\\?\\UNC\\") {
        format!("\\\\{value}")
    } else if let Some(value) = raw.strip_prefix("\\\\?\\") {
        value.to_string()
    } else {
        raw
    }
}

fn runtime_http_blocking_client() -> Result<reqwest::blocking::Client, String> {
    let mut builder = reqwest::blocking::Client::builder()
        .http1_only()
        .connect_timeout(Duration::from_secs(RUNTIME_HTTP_CONNECT_TIMEOUT_SECONDS))
        .timeout(Duration::from_secs(RUNTIME_HTTP_REQUEST_TIMEOUT_SECONDS));
    if let Some(proxy_url) = configured_proxy_url_best_effort() {
        let proxy =
            reqwest::Proxy::all(&proxy_url).map_err(|err| format!("代理配置无效：{err}"))?;
        builder = builder.proxy(proxy);
    }
    builder
        .build()
        .map_err(|err| format!("无法初始化运行环境下载客户端：{err}"))
}

async fn test_github_connectivity_via_proxy(
    proxy_url: &str,
    http1_only: bool,
) -> Result<ProxyTestResult, String> {
    test_github_connectivity_via_proxy_with_timeout(
        proxy_url,
        http1_only,
        Duration::from_secs(PROXY_TEST_TIMEOUT_SECONDS),
    )
    .await
}

async fn test_github_connectivity_via_proxy_with_timeout(
    proxy_url: &str,
    http1_only: bool,
    timeout: Duration,
) -> Result<ProxyTestResult, String> {
    let mut builder = reqwest::Client::builder()
        .connect_timeout(timeout)
        .timeout(timeout);
    if http1_only {
        builder = builder.http1_only();
    }
    let client = apply_reqwest_proxy(builder, Some(proxy_url))?
        .build()
        .map_err(|err| format!("无法初始化代理测试客户端：{err}"))?;
    let started_at = Instant::now();
    let response = client
        .get(GITHUB_CONNECTIVITY_TEST_URL)
        .header("User-Agent", "BiblioSmith-Launcher")
        .send()
        .await
        .map_err(|err| format!("{err}"))?;
    let elapsed_ms = started_at.elapsed().as_millis();
    let status = response.status();
    let version = format!("{:?}", response.version());
    let outcome = github_connectivity_outcome(status, elapsed_ms, &version);
    if !outcome.ok {
        return Err(outcome.message);
    }
    Ok(outcome)
}

fn github_connectivity_outcome(
    status: StatusCode,
    elapsed_ms: u128,
    http_version: &str,
) -> ProxyTestResult {
    let ok = status.is_success()
        || matches!(
            status,
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS | StatusCode::UNAUTHORIZED
        );
    let message = if status.is_success() {
        format!("代理可连接 GitHub，耗时 {elapsed_ms} ms。")
    } else if ok {
        format!(
            "GitHub 已响应（HTTP {status}），代理链路可用，耗时 {elapsed_ms} ms。若更新仍失败，请查看 Git 分支状态、权限或限流信息。"
        )
    } else {
        format!("GitHub 返回 HTTP status {status}，代理未通过连通性测试。")
    };
    ProxyTestResult {
        ok,
        message,
        elapsed_ms: Some(elapsed_ms),
        http_version: Some(http_version.to_string()),
        target_url: GITHUB_CONNECTIVITY_TEST_URL.into(),
    }
}

fn proxy_test_failure_result(message: String) -> ProxyTestResult {
    ProxyTestResult {
        ok: false,
        message,
        elapsed_ms: None,
        http_version: None,
        target_url: GITHUB_CONNECTIVITY_TEST_URL.into(),
    }
}

fn launcher_current_version() -> String {
    format!("v{}", env!("CARGO_PKG_VERSION"))
}

fn is_proxy_configured() -> bool {
    if configured_proxy_url_best_effort().is_some() {
        return true;
    }
    [
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "ALL_PROXY",
        "https_proxy",
        "http_proxy",
        "all_proxy",
    ]
    .iter()
    .any(|key| {
        std::env::var(key)
            .map(|value| !value.trim().is_empty())
            .unwrap_or(false)
    })
}

fn download_percent(downloaded: u64, total: u64) -> f64 {
    if downloaded == 0 {
        return 0.0;
    }
    if total == 0 {
        return 1.0;
    }
    clamp_progress_percent(((downloaded as f64 / total as f64) * 100.0).clamp(1.0, 100.0))
}

fn show_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn hide_main_window(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.hide();
    }
}

fn configure_tray(app: &mut tauri::App) -> tauri::Result<()> {
    let tray_menu = MenuBuilder::new(app)
        .text(TRAY_SHOW_ID, "打开 BiblioSmith Launcher")
        .text(TRAY_HIDE_ID, "隐藏窗口")
        .separator()
        .text(TRAY_QUIT_ID, "退出 BiblioSmith Launcher")
        .build()?;
    let mut tray = TrayIconBuilder::with_id("bibliosmith-launcher")
        .tooltip("BiblioSmith Launcher")
        .menu(&tray_menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_SHOW_ID => show_main_window(app),
            TRAY_HIDE_ID => hide_main_window(app),
            TRAY_QUIT_ID => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| match event {
            TrayIconEvent::Click {
                button: MouseButton::Left,
                ..
            }
            | TrayIconEvent::DoubleClick {
                button: MouseButton::Left,
                ..
            } => show_main_window(tray.app_handle()),
            _ => {}
        });
    if let Some(icon) = app.default_window_icon().cloned() {
        tray = tray.icon(icon);
    }
    tray.build(app)?;
    Ok(())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    install_panic_log_hook();
    tauri::Builder::default()
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![]),
        ))
        .setup(|app| {
            app_paths::initialize(app)?;
            let window = app.get_webview_window("main").expect("main window missing");
            let _ = window.set_title(&format!(
                "BiblioSmith Launcher {}",
                launcher_current_version()
            ));
            append_launcher_log(
                "INFO",
                format!(
                    "BiblioSmith Launcher {} started log_path={}",
                    launcher_current_version(),
                    launcher_log_path()
                        .map(|path| display_path(&path))
                        .unwrap_or_else(|error| error)
                ),
            );
            set_process_runtime_envs();
            configure_tray(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_workspace_state,
            create_recommended_workspace,
            choose_and_create_workspace,
            get_diagnostic_log_settings,
            set_save_logs_enabled,
            get_proxy_settings,
            save_proxy_settings,
            test_proxy_settings,
            auto_detect_proxy_settings,
            model_settings::get_model_catalog,
            model_settings::save_model_credential,
            model_settings::save_qwen_settings,
            model_settings::delete_model_credential,
            model_settings::set_active_model,
            model_settings::test_model_connection,
            embedding_settings::get_embedding_status,
            embedding_settings::save_embedding_credential,
            embedding_settings::delete_embedding_credential,
            embedding_settings::test_embedding_connection,
            ocr_settings::get_ocr_credentials_status,
            ocr_settings::save_ocr_credential,
            ocr_settings::delete_ocr_credential,
            ocr_settings::test_ocr_connection,
            zotero_settings::get_zotero_credentials_status,
            zotero_settings::save_zotero_credential,
            zotero_settings::delete_zotero_credential,
            get_runtime_status,
            start_runtime_prepare,
            export_launcher_logs,
            record_frontend_activity,
            minimize_main_window,
            toggle_main_window_maximized,
            close_main_window_to_tray,
            book_pipeline::get_book_pipeline_state,
            book_pipeline::preview_book_pipeline_route,
            book_pipeline::queue_book_pipeline_job,
            book_pipeline::save_book_pipeline_custom_instructions,
            book_pipeline::get_book_pipeline_structure_correction_draft,
            book_pipeline::save_book_pipeline_structure_correction,
            book_pipeline::run_book_pipeline_job,
            book_pipeline::retry_book_pipeline_job,
            book_pipeline::remove_books_from_shelf,
            book_pipeline::inspect_book_pipeline_project_migration,
            book_pipeline::migrate_book_pipeline_project,
            book_pipeline::advance_book_pipeline_job,
            book_pipeline::approve_book_pipeline_gate,
            book_pipeline::record_book_pipeline_reader_evidence,
            book_pipeline::set_book_pipeline_route_override,
            book_pipeline::verify_book_pipeline_cleanup_approval,
            book_pipeline::run_book_pipeline_translation_sample,
            book_pipeline::set_book_pipeline_translation_provider,
            book_pipeline::choose_book_pipeline_pdf_folder,
            book_pipeline::choose_book_pipeline_markdown_source,
            book_pipeline::discover_book_pipeline_zotero_sources,
            book_pipeline::handoff_book_pipeline_markdown,
            book_pipeline::preview_book_pipeline_cleanup,
            book_pipeline::approve_book_pipeline_cleanup,
            book_pipeline::export_book_pipeline_diagnostic,
            book_pipeline::save_book_pipeline_diagnostic,
            book_pipeline::open_book_pipeline_output,
            book_pipeline::read_book_pipeline_artifact_excerpt,
            book_pipeline::read_book_pipeline_translation_sample,
            book_pipeline::run_book_pipeline_ocr_sample,
            book_pipeline::read_book_pipeline_ocr_sample,
            book_pipeline::attach_book_pipeline_artifact_to_zotero
        ])
        .build(tauri::generate_context!())
        .expect("error while running BiblioSmith Launcher")
        .run(|app, event| {
            // macOS: clicking the Dock icon should bring back the tray-hidden window.
            if let tauri::RunEvent::Reopen { .. } = event {
                show_main_window(app);
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_test_path(name: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock should be valid")
            .as_nanos();
        env::temp_dir().join(format!("bibliosmith-launcher-{name}-{suffix}"))
    }

    #[test]
    fn launcher_config_paths_separate_development_from_release() {
        let base = Path::new("config-base");

        assert_eq!(
            launcher_config_path_from_base(base, true),
            base.join("BiblioSmith")
                .join("launcher-dev")
                .join("config.json")
        );
        assert_eq!(
            launcher_config_path_from_base(base, false),
            base.join("BiblioSmith")
                .join("launcher")
                .join("config.json")
        );
    }

    #[test]
    fn workspace_state_uses_documents_default_and_never_requires_git() {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = app_paths::AppPaths::from_roots(
            temp.path()
                .join("App.app/Contents/Resources/bibliosmith-runtime"),
            temp.path()
                .join("Library/Application Support/BiblioSmith/launcher"),
            temp.path().join("Library/Caches/BiblioSmith/launcher"),
            temp.path().join("Documents"),
        );

        let state = collect_workspace_state_from(&paths, None);

        assert_eq!(
            state.workspace_root,
            display_path(&temp.path().join("Documents/BiblioSmith"))
        );
        assert_eq!(state.recommended_workspace_root, state.workspace_root);
        assert!(!state.workspace_ready);
        assert_eq!(state.workspace_status, app_paths::WorkspaceStatus::Missing);
    }

    #[test]
    fn launcher_config_ignores_the_retired_repo_root_contract() {
        let legacy: LauncherConfig = serde_json::from_value(serde_json::json!({
            "repoRoot": "/developer/checkout"
        }))
        .expect("legacy config remains parseable as unknown input");
        let current: LauncherConfig = serde_json::from_value(serde_json::json!({
            "workspaceRoot": "/test-data/Documents/BiblioSmith"
        }))
        .expect("workspace config");

        assert_eq!(configured_workspace_root_from_config(Some(&legacy)), None);
        assert_eq!(
            configured_workspace_root_from_config(Some(&current)),
            Some(PathBuf::from("/test-data/Documents/BiblioSmith"))
        );
    }

    #[test]
    fn runtime_packages_use_private_zip_fallbacks_and_sha256() {
        let packages = runtime_packages();

        assert_eq!(packages.len(), 2);
        for package in packages {
            assert!(
                package.urls.len() >= 2,
                "{} runtime should not depend on a single download source",
                package.kind.label()
            );
            assert_eq!(package.sha256.len(), 64);
            assert!(package.sha256.chars().all(|ch| ch.is_ascii_hexdigit()));
            assert!(package.archive_name.ends_with(".zip"));
            assert!(package.size_bytes > 1024 * 1024);
        }
        for checksum in [PYTHON_RUNTIME_SHA256]
            .into_iter()
            .chain(JAVA_RUNTIME_SHA256S)
        {
            assert_eq!(checksum.len(), 64);
            assert!(checksum.chars().all(|ch| ch.is_ascii_hexdigit()));
        }
    }

    #[test]
    fn runtime_download_detail_uses_kb_percent_and_speed() {
        let detail = runtime_download_detail(768 * 1024, 2048 * 1024, Duration::from_secs(3));

        assert_eq!(detail, "37.50% (768.0 KB / 2048.0 KB, 256.0 KB/s)");
        assert!(!detail.contains("MB"));
        assert!(!detail.contains("MiB"));
    }

    #[test]
    fn runtime_private_paths_stay_under_bibliosmith_runtime_root() {
        let root = temp_test_path("runtime-root");
        let package = runtime_packages()[0];
        let dir = runtime_install_dir_from_root(&root, package);

        assert!(dir.starts_with(root.join(package.kind.dir_name())));
        assert!(dir.ends_with(package.install_dir_name));
    }

    #[test]
    fn runtime_download_archives_stay_in_the_cache_layer() {
        let root = temp_test_path("runtime-download-cache");
        let paths = app_paths::AppPaths::from_roots(
            root.join("App.app/Contents/Resources/bibliosmith-runtime"),
            root.join("Library/Application Support/BiblioSmith/launcher"),
            root.join("Library/Caches/BiblioSmith/launcher"),
            root.join("Documents"),
        );

        let downloads = runtime_downloads_dir_for(&paths);

        assert_eq!(
            downloads,
            root.join("Library/Caches/BiblioSmith/launcher/runtimes/downloads")
        );
        assert!(!downloads.starts_with(paths.support_root()));
    }

    #[test]
    fn system_ready_runtime_does_not_require_private_download() {
        let python = RuntimeToolStatus {
            ready: true,
            private_ready: false,
            version: PYTHON_RUNTIME_VERSION.into(),
            source: Some("system".into()),
            path: Some(r"C:\Python313\python.exe".into()),
            message: "Python system runtime is available.".into(),
        };
        let java = RuntimeToolStatus {
            ready: true,
            private_ready: false,
            version: JAVA_RUNTIME_VERSION.into(),
            source: Some("system".into()),
            path: Some(r"C:\Program Files\Java\bin\java.exe".into()),
            message: "Java system runtime is available.".into(),
        };
        let status = RuntimeStatus {
            ready: true,
            private_ready: false,
            running: false,
            runtime_root: r"C:\Users\tester\AppData\Local\BiblioSmith\runtimes".into(),
            python,
            java,
        };

        assert!(!runtime_prepare_requires_download(&status));
    }

    #[test]
    fn java_home_runtime_path_is_supported_without_path_lookup() {
        let root = temp_test_path("java-home-runtime");
        let bin = root.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let java = bin.join(java_executable_name());
        fs::write(&java, "test").unwrap();

        assert_eq!(java_home_executable_from_value(&root), Some(java));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn download_percent_reports_visible_progress_after_first_chunk() {
        assert_eq!(download_percent(0, 100), 0.0);
        assert_eq!(download_percent(1, 100_000_000), 1.0);
        assert_eq!(download_percent(1_234, 100_000), 1.23);
        assert_eq!(download_percent(50, 100), 50.0);
        assert_eq!(download_percent(100, 100), 100.0);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_expand_archive_command_uses_environment_paths() {
        let script = windows_expand_archive_command_script();

        assert!(script.contains("$env:BIBLIOSMITH_ARCHIVE_ZIP"));
        assert!(script.contains("$env:BIBLIOSMITH_ARCHIVE_DEST"));
        assert!(!script.contains("$args"));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_zip_extract_falls_back_to_tar_when_powershell_cannot_start() {
        let root = temp_test_path("archive-powershell-start-fallback");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let archive_file = root.join("missing.zip");
        let destination = root.join("dest");

        let error = extract_zip_archive_with_windows_tools(
            &archive_file,
            &destination,
            &[PathBuf::from("__bibliosmith_missing_powershell__.exe")],
            Path::new("__bibliosmith_missing_tar__.exe"),
        )
        .expect_err("missing PowerShell must continue to tar and then report both failures");

        assert!(error.contains("PowerShell"));
        assert!(error.contains("tar"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn archive_entry_paths_strip_github_root_and_reject_traversal() {
        assert_eq!(
            safe_archive_entry_relative_path("bibliosmith-main/AGENTS.md").unwrap(),
            PathBuf::from("AGENTS.md")
        );
        assert_eq!(
            safe_archive_entry_relative_path("bibliosmith-main/templates/local-reading/README.md")
                .unwrap(),
            PathBuf::from("templates")
                .join("local-reading")
                .join("README.md")
        );
        assert!(safe_archive_entry_relative_path("bibliosmith-main/../evil.txt").is_err());
        assert!(safe_archive_entry_relative_path("single-root-only").is_err());
    }

    #[test]
    fn diagnostic_logging_defaults_to_enabled_for_missing_config() {
        assert!(diagnostic_logging_enabled_from_config(
            &LauncherConfig::default()
        ));
    }

    #[test]
    fn diagnostic_logging_can_be_disabled_from_config() {
        let config = LauncherConfig {
            workspace_root: None,
            save_logs: Some(false),
            proxy: None,
            active_model: None,
            qwen_workspace_id: None,
            qwen_web_search_enabled: None,
        };

        assert!(!diagnostic_logging_enabled_from_config(&config));
    }

    #[test]
    fn proxy_url_requires_host_and_port_when_enabled() {
        let proxy = NetworkProxySettings {
            enabled: true,
            scheme: "socks5h".into(),
            host: "127.0.0.1".into(),
            port: Some(10808),
        };
        assert_eq!(
            proxy_url_from_settings(&proxy).unwrap(),
            Some("socks5h://127.0.0.1:10808".into())
        );

        let missing_host = NetworkProxySettings {
            host: " ".into(),
            ..proxy.clone()
        };
        assert!(proxy_url_from_settings(&missing_host).is_err());
    }

    #[test]
    fn proxy_settings_from_url_accepts_common_local_proxy_urls() {
        let http =
            proxy_settings_from_url("http://127.0.0.1:7890").expect("HTTP proxy URL should parse");
        assert!(http.enabled);
        assert_eq!(http.scheme, "http");
        assert_eq!(http.host, "127.0.0.1");
        assert_eq!(http.port, Some(7890));

        let socks = proxy_settings_from_url("socks5h://localhost:10808")
            .expect("SOCKS proxy URL should parse");
        assert_eq!(socks.scheme, "socks5h");
        assert_eq!(socks.host, "localhost");
        assert_eq!(socks.port, Some(10808));

        let wininet_socks = proxy_settings_from_url("socks=127.0.0.1:10808")
            .expect("Windows SOCKS proxy entry should parse as SOCKS");
        assert_eq!(wininet_socks.scheme, "socks5");
        assert_eq!(wininet_socks.host, "127.0.0.1");
        assert_eq!(wininet_socks.port, Some(10808));

        let no_scheme = proxy_settings_from_url("127.0.0.1:7897")
            .expect("host:port proxy should default to HTTP");
        assert_eq!(no_scheme.scheme, "http");
        assert_eq!(no_scheme.port, Some(7897));

        assert!(proxy_settings_from_url("not-a-valid-proxy").is_none());
    }

    #[test]
    fn github_connectivity_status_treats_forbidden_as_reachable() {
        let outcome = github_connectivity_outcome(StatusCode::FORBIDDEN, 429, "HTTP/2");

        assert!(outcome.ok);
        assert!(outcome.message.contains("GitHub 已响应"));
        assert_eq!(outcome.elapsed_ms, Some(429));
        assert_eq!(outcome.http_version.as_deref(), Some("HTTP/2"));
    }

    #[test]
    fn proxy_detection_candidates_ignore_current_manual_proxy() {
        let current = NetworkProxySettings {
            enabled: true,
            scheme: "http".into(),
            host: "2".into(),
            port: Some(2),
        };

        let candidates = proxy_detection_candidates_with_current(&current);

        assert!(!candidates
            .iter()
            .any(|candidate| candidate.host == "2" && candidate.port == Some(2)));
    }

    #[test]
    fn proxy_detection_candidates_prefer_socks_for_common_10808_port() {
        let candidates = static_proxy_detection_candidates();
        let socks_index = candidates
            .iter()
            .position(|candidate| {
                candidate.scheme == "socks5h"
                    && candidate.host == "127.0.0.1"
                    && candidate.port == Some(10808)
            })
            .expect("common SOCKS proxy candidate should be present");
        let http_index = candidates
            .iter()
            .position(|candidate| {
                candidate.scheme == "http"
                    && candidate.host == "127.0.0.1"
                    && candidate.port == Some(10808)
            })
            .expect("common HTTP proxy candidate should be present");

        assert!(socks_index < http_index);
    }

    #[test]
    fn rotating_diagnostic_log_keeps_newest_files_and_removes_oldest() {
        let dir = temp_test_path("diagnostic-rotation");
        fs::create_dir_all(&dir).expect("log directory should be created");
        let log_path = dir.join("bibliosmith-launcher.log");

        append_launcher_log_to_path(&log_path, true, 24, 2, "INFO", "first-line")
            .expect("first log write should succeed");
        append_launcher_log_to_path(&log_path, true, 24, 2, "INFO", "second-line")
            .expect("second log write should rotate");
        append_launcher_log_to_path(&log_path, true, 24, 2, "INFO", "third-line")
            .expect("third log write should rotate");

        assert!(
            fs::read_to_string(&log_path)
                .expect("current log should exist")
                .contains("third-line"),
            "current log should contain newest line"
        );
        assert!(
            fs::read_to_string(log_path.with_extension("log.1"))
                .expect("first rotated log should exist")
                .contains("second-line"),
            "first backup should contain the previous line"
        );
        assert!(
            fs::read_to_string(log_path.with_extension("log.2"))
                .expect("second rotated log should exist")
                .contains("first-line"),
            "second backup should contain the oldest retained line"
        );
        assert!(
            !log_path.with_extension("log.3").exists(),
            "rotation should cap backup count"
        );

        fs::remove_dir_all(&dir).expect("test log directory should be cleaned");
    }

    #[test]
    fn disabled_diagnostic_logging_does_not_create_log_file() {
        let dir = temp_test_path("diagnostic-disabled");
        fs::create_dir_all(&dir).expect("log directory should be created");
        let log_path = dir.join("bibliosmith-launcher.log");

        append_launcher_log_to_path(&log_path, false, 1024, 2, "INFO", "hidden-line")
            .expect("disabled log write should still return ok");

        assert!(!log_path.exists(), "disabled logging must not create files");

        fs::remove_dir_all(&dir).expect("test log directory should be cleaned");
    }

    #[test]
    fn default_diagnostic_log_policy_caps_single_file_at_five_mb() {
        assert_eq!(LAUNCHER_LOG_MAX_BYTES, 5 * 1024 * 1024);
        assert_eq!(LAUNCHER_LOG_BACKUP_COUNT, 0);
    }

    #[test]
    fn export_diagnostic_logs_copies_rotated_logs_and_context() {
        let log_dir = temp_test_path("diagnostic-export-source");
        let export_parent = temp_test_path("diagnostic-export-target");
        fs::create_dir_all(&log_dir).expect("log directory should be created");
        fs::create_dir_all(&export_parent).expect("export directory should be created");
        fs::write(log_dir.join("bibliosmith-launcher.log"), "current")
            .expect("current log written");
        fs::write(log_dir.join("bibliosmith-launcher.log.1"), "previous")
            .expect("rotated log written");

        let context = diagnostic_context_for_export(
            "v-test",
            "D:\\BiblioSmith",
            app_paths::WorkspaceStatus::Ready,
            true,
            &log_dir,
            4096,
            2,
        );
        let export_dir = export_diagnostic_logs_to_dir(&export_parent, &log_dir, &context)
            .expect("diagnostic logs should export");

        assert!(export_dir.join("bibliosmith-launcher.log").is_file());
        assert!(export_dir.join("bibliosmith-launcher.log.1").is_file());
        let context_text = fs::read_to_string(export_dir.join("diagnostic-context.json"))
            .expect("diagnostic context should be exported");
        assert!(context_text.contains("\"workspaceRoot\": \"D:\\\\BiblioSmith\""));
        assert!(context_text.contains("\"workspaceStatus\": \"ready\""));
        assert!(context_text.contains("\"saveLogs\": true"));

        fs::remove_dir_all(&log_dir).expect("source log directory should be cleaned");
        fs::remove_dir_all(&export_parent).expect("export directory should be cleaned");
    }
}
