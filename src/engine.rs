use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use regex::Regex;
use reqwest::{Client, RequestBuilder};
use serde_json::Value;
use tokio::sync::{Mutex, Notify};
use tokio::time;

pub struct Executor {
    pub cyrus_binary_path: Option<PathBuf>,
    pub last_run_id: Option<String>,
    pub initialized: bool,
    pub ready: Arc<Notify>,
    pub download_error: Option<String>,
    pub github_token: String,
}

impl Executor {
    pub fn new(github_token: String) -> Self {
        Self {
            cyrus_binary_path: None,
            last_run_id: None,
            initialized: false,
            ready: Arc::new(Notify::new()),
            download_error: None,
            github_token,
        }
    }
}

pub struct ExecutionResult {
    pub success: bool,
    pub stdout: String,
    pub stderr: String,
    pub execution_time: f64,
}

pub async fn execute_cyrus_code(
    executor: Arc<Mutex<Executor>>,
    code: &str,
) -> Result<ExecutionResult, String> {
    let binary_path = loop {
        let notified = {
            let lock = executor.lock().await;

            if let Some(path) = &lock.cyrus_binary_path {
                if path.exists() {
                    break path.clone();
                }
            }

            if lock.initialized {
                let error_msg = lock
                    .download_error
                    .as_deref()
                    .unwrap_or(
                        "Cyrus binary is unavailable. The latest build could not be downloaded.",
                    )
                    .to_string();

                return Err(error_msg);
            }

            lock.ready.clone().notified_owned()
        };

        log::debug!("Waiting for Cyrus binary to become ready...");
        notified.await;
    };

    let mut temp_file = tempfile::Builder::new()
        .suffix(".cyrus")
        .tempfile()
        .map_err(|e| format!("Failed to create temp file: {e}"))?;

    temp_file
        .as_file_mut()
        .write_all(code.as_bytes())
        .map_err(|e| format!("Failed to write code: {e}"))?;

    let start = std::time::Instant::now();

    let stdlib_path = binary_path
        .parent()
        .map(|p| p.join("stdlib"))
        .filter(|p| p.exists());

    let mut cmd = Command::new(&binary_path);

    cmd.arg("run").arg(temp_file.path());

    if let Some(stdlib) = stdlib_path {
        cmd.arg("--stdlib").arg(stdlib);
    }

    let output = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Failed to execute Cyrus: {e}"))?;

    let elapsed = start.elapsed();

    Ok(ExecutionResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        execution_time: elapsed.as_secs_f64(),
    })
}

pub async fn download_latest_cyrus(executor: Arc<Mutex<Executor>>) -> Result<PathBuf, String> {
    const REPO_OWNER: &str = "cyrus-lang";
    const REPO_NAME: &str = "Cyrus";
    const WORKFLOW: &str = "build-linux.yml";
    const BRANCH: &str = "main";

    let (github_token, client) = {
        let lock = executor.lock().await;

        let token = lock.github_token.trim();

        if token.is_empty() {
            return Err(
                "GitHub token is empty. Set the GITHUB_TOKEN environment variable.".to_string(),
            );
        }

        let client = Client::builder()
            .user_agent("cyrus-playground")
            .build()
            .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

        (token.to_string(), client)
    };

    let artifact_regex =
        Regex::new(r"^cyrus-v?\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?-binary$")
            .map_err(|e| format!("Failed to create artifact name regex: {e}"))?;

    log::info!("Discovering latest successful Cyrus build...");

    let runs_url = format!(
        "https://api.github.com/repos/{}/{}/actions/workflows/{}/runs",
        REPO_OWNER, REPO_NAME, WORKFLOW
    );

    let runs_response = github_get(&client, &github_token, &runs_url)
        .query(&[("branch", BRANCH), ("status", "success"), ("per_page", "1")])
        .send()
        .await
        .map_err(|e| format!("Failed to query workflow runs: {e}"))?;

    if !runs_response.status().is_success() {
        let status = runs_response.status();
        let error_body = runs_response
            .text()
            .await
            .unwrap_or_else(|_| "No error body".to_string());

        return Err(format!(
            "Failed to query workflow runs: HTTP {} - {}",
            status, error_body
        ));
    }

    let runs_json: Value = runs_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse workflow runs response: {e}"))?;

    let workflow_runs = runs_json["workflow_runs"]
        .as_array()
        .ok_or_else(|| "Workflow runs response does not contain workflow_runs".to_string())?;

    let latest_run = workflow_runs
        .first()
        .ok_or_else(|| "No successful Cyrus workflow runs were found".to_string())?;

    let run_id = latest_run["id"]
        .as_u64()
        .ok_or_else(|| "Latest workflow run has no valid ID".to_string())?;

    let run_id_string = run_id.to_string();

    let head_sha = latest_run["head_sha"].as_str().unwrap_or("unknown");

    let created_at = latest_run["created_at"].as_str().unwrap_or("unknown");

    log::info!(
        "Latest successful Cyrus build: run_id={}, commit={}, created_at={}",
        run_id,
        head_sha,
        created_at
    );

    {
        let lock = executor.lock().await;

        if lock.last_run_id.as_deref() == Some(run_id_string.as_str()) {
            if let Some(path) = &lock.cyrus_binary_path {
                if path.exists() {
                    log::info!(
                        "Cyrus binary is already up to date (workflow run {})",
                        run_id
                    );

                    return Ok(path.clone());
                }
            }
        }
    }

    let artifacts_url = format!(
        "https://api.github.com/repos/{}/{}/actions/runs/{}/artifacts",
        REPO_OWNER, REPO_NAME, run_id
    );

    let artifacts_response = github_get(&client, &github_token, &artifacts_url)
        .query(&[("per_page", "100")])
        .send()
        .await
        .map_err(|e| format!("Failed to query workflow artifacts: {e}"))?;

    if !artifacts_response.status().is_success() {
        let status = artifacts_response.status();
        let error_body = artifacts_response
            .text()
            .await
            .unwrap_or_else(|_| "No error body".to_string());

        return Err(format!(
            "Failed to query workflow artifacts: HTTP {} - {}",
            status, error_body
        ));
    }

    let artifacts_json: Value = artifacts_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse workflow artifacts response: {e}"))?;

    let artifacts = artifacts_json["artifacts"]
        .as_array()
        .ok_or_else(|| "Workflow artifacts response does not contain artifacts".to_string())?;

    let artifact = artifacts
    .iter()
    .filter(|artifact| {
        artifact["expired"]
            .as_bool()
            .map(|expired| !expired)
            .unwrap_or(false)
    })
    .find(|artifact| {
        artifact["name"]
            .as_str()
            .map(|name| artifact_regex.is_match(name))
            .unwrap_or(false)
    })
    .ok_or_else(|| {
        let available = artifacts
            .iter()
            .filter_map(|artifact| artifact["name"].as_str())
            .collect::<Vec<_>>()
            .join(", ");

        format!(
            "No Cyrus binary artifact matching the expected pattern was found in workflow run {}. Available artifacts: {}",
            run_id,
            if available.is_empty() {
                "none".to_string()
            } else {
                available
            }
        )
    })?;

    let artifact_id = artifact["id"]
        .as_u64()
        .ok_or_else(|| "Selected workflow artifact has no valid ID".to_string())?;

    let artifact_name = artifact["name"]
        .as_str()
        .ok_or_else(|| "Selected workflow artifact has no name".to_string())?
        .to_string();

    let artifact_version = artifact_name
        .strip_prefix("cyrus-")
        .unwrap_or(&artifact_name)
        .strip_suffix("-binary")
        .unwrap_or(&artifact_name)
        .to_string();

    log::info!(
        "Selected Cyrus artifact: name={}, id={}, version={}",
        artifact_name,
        artifact_id,
        artifact_version
    );

    let download_url = format!(
        "https://api.github.com/repos/{}/{}/actions/artifacts/{}/zip",
        REPO_OWNER, REPO_NAME, artifact_id
    );

    log::info!(
        "Downloading Cyrus artifact from GitHub Actions: {}",
        download_url
    );

    let artifact_response = github_get(&client, &github_token, &download_url)
        .send()
        .await
        .map_err(|e| format!("Failed to download Cyrus artifact: {e}"))?;

    if !artifact_response.status().is_success() {
        let status = artifact_response.status();
        let error_body = artifact_response
            .text()
            .await
            .unwrap_or_else(|_| "No error body".to_string());

        return Err(format!(
            "Cyrus artifact download failed: HTTP {} - {}",
            status, error_body
        ));
    }

    let bytes = artifact_response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read Cyrus artifact bytes: {e}"))?;

    install_artifact(
        bytes,
        executor,
        &run_id_string,
        &artifact_name,
        &artifact_version,
    )
    .await
}

fn github_get(client: &Client, token: &str, url: &str) -> RequestBuilder {
    client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .header("Authorization", format!("Bearer {}", token))
        .header("X-GitHub-Api-Version", "2026-03-10")
}

async fn install_artifact(
    bytes: Bytes,
    executor: Arc<Mutex<Executor>>,
    run_id: &str,
    artifact_name: &str,
    version: &str,
) -> Result<PathBuf, String> {
    let temp_dir =
        tempfile::tempdir().map_err(|e| format!("Failed to create temporary directory: {e}"))?;

    let temp_zip = temp_dir.path().join("artifact.zip");

    fs::write(&temp_zip, &bytes).map_err(|e| format!("Failed to write artifact zip: {e}"))?;

    let file =
        fs::File::open(&temp_zip).map_err(|e| format!("Failed to open artifact zip: {e}"))?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid artifact ZIP: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry at index {}: {e}", i))?;

        let relative_path = PathBuf::from(file.name());

        if relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!("Unsafe path in artifact: {}", file.name()));
        }

        let outpath = temp_dir.path().join(&relative_path);

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath)
                .map_err(|e| format!("Failed to create directory {}: {e}", outpath.display()))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| {
                    format!(
                        "Failed to create parent directory {}: {e}",
                        parent.display()
                    )
                })?;
            }

            let mut outfile = fs::File::create(&outpath).map_err(|e| {
                format!("Failed to create extracted file {}: {e}", outpath.display())
            })?;

            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract file {}: {e}", outpath.display()))?;
        }
    }

    let extracted_binary = find_cyrus_binary(&temp_dir.path().to_path_buf()).map_err(|e| {
        format!(
            "Downloaded artifact {} does not contain Cyrus binary: {e}",
            artifact_name
        )
    })?;

    log::info!("Downloaded Cyrus binary: {:?}", extracted_binary);

    let current_dir =
        std::env::current_dir().map_err(|e| format!("Failed to get current directory: {e}"))?;

    let extract_dir = current_dir.join("cyrus_bin");
    let new_dir = current_dir.join("cyrus_bin.new");

    if new_dir.exists() {
        fs::remove_dir_all(&new_dir).map_err(|e| {
            format!(
                "Failed to remove old temporary installation at {}: {e}",
                new_dir.display()
            )
        })?;
    }

    fs::create_dir_all(&new_dir).map_err(|e| {
        format!(
            "Failed to create installation directory {}: {e}",
            new_dir.display()
        )
    })?;

    copy_dir_recursive(temp_dir.path(), &new_dir)
        .map_err(|e| format!("Failed to install artifact: {e}"))?;

    let new_binary = find_cyrus_binary(&new_dir)
        .map_err(|e| format!("Installed artifact does not contain Cyrus binary: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(&new_binary)
            .map_err(|e| {
                format!(
                    "Failed to stat Cyrus binary at {}: {e}",
                    new_binary.display()
                )
            })?
            .permissions();

        perms.set_mode(0o755);

        fs::set_permissions(&new_binary, perms)
            .map_err(|e| format!("Failed to set Cyrus permissions: {e}"))?;
    }

    fs::write(new_dir.join(".version"), version)
        .map_err(|e| format!("Failed to write version metadata: {e}"))?;

    fs::write(new_dir.join(".run_id"), run_id)
        .map_err(|e| format!("Failed to write workflow run metadata: {e}"))?;

    fs::write(new_dir.join(".artifact"), artifact_name)
        .map_err(|e| format!("Failed to write artifact metadata: {e}"))?;

    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir).map_err(|e| {
            format!(
                "Failed to remove old Cyrus installation at {}: {e}",
                extract_dir.display()
            )
        })?;
    }

    fs::rename(&new_dir, &extract_dir)
        .map_err(|e| format!("Failed to install new Cyrus binary: {e}",))?;

    let binary_path = find_cyrus_binary(&extract_dir)
        .map_err(|e| format!("Installed Cyrus binary cannot be found: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(&binary_path)
            .map_err(|e| {
                format!(
                    "Failed to stat installed Cyrus binary at {}: {e}",
                    binary_path.display()
                )
            })?
            .permissions();

        perms.set_mode(0o755);

        fs::set_permissions(&binary_path, perms)
            .map_err(|e| format!("Failed to set installed Cyrus permissions: {e}"))?;
    }

    let notify = {
        let mut lock = executor.lock().await;

        lock.cyrus_binary_path = Some(binary_path.clone());
        lock.last_run_id = Some(run_id.to_string());
        lock.initialized = true;
        lock.download_error = None;

        lock.ready.clone()
    };

    notify.notify_waiters();

    log::info!(
        "Cyrus binary updated successfully: version={}, run_id={}, artifact={}",
        version,
        run_id,
        artifact_name
    );

    Ok(binary_path)
}

pub async fn auto_update_cyrus(executor: Arc<Mutex<Executor>>) {
    log::info!("Starting Cyrus auto-update task");

    let extract_dir = match std::env::current_dir() {
        Ok(dir) => dir.join("cyrus_bin"),

        Err(e) => {
            log::error!("Failed to get current directory: {}", e);

            let notify = {
                let mut lock = executor.lock().await;

                lock.initialized = true;
                lock.download_error = Some(format!("Failed to get current directory: {}", e));

                lock.ready.clone()
            };

            notify.notify_waiters();

            return;
        }
    };

    if extract_dir.exists() {
        match find_cyrus_binary(&extract_dir) {
            Ok(binary_path) => {
                log::info!("Found existing Cyrus installation: {:?}", binary_path);

                let run_id = fs::read_to_string(extract_dir.join(".run_id"))
                    .ok()
                    .map(|s| s.trim().to_string());

                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;

                    if let Ok(metadata) = fs::metadata(&binary_path) {
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o755);

                        if let Err(e) = fs::set_permissions(&binary_path, perms) {
                            log::warn!("Failed to set Cyrus executable permissions: {}", e);
                        }
                    }
                }

                let notify = {
                    let mut lock = executor.lock().await;

                    lock.cyrus_binary_path = Some(binary_path.clone());
                    lock.last_run_id = run_id;

                    lock.ready.clone()
                };

                notify.notify_waiters();

                log::info!("Existing Cyrus binary restored successfully");
            }

            Err(e) => {
                log::info!("No valid cached Cyrus binary found: {}", e);
            }
        }
    }

    let download_result = download_latest_cyrus(executor.clone()).await;

    match download_result {
        Ok(path) => {
            log::info!("Initial Cyrus update check completed: {:?}", path);
        }

        Err(e) => {
            log::error!("Initial Cyrus update failed: {}", e);

            let notify = {
                let mut lock = executor.lock().await;

                lock.download_error = Some(e.clone());
                lock.initialized = true;

                lock.ready.clone()
            };

            notify.notify_waiters();
        }
    }

    let notify = {
        let mut lock = executor.lock().await;

        if !lock.initialized {
            lock.initialized = true;
        }

        lock.ready.clone()
    };

    notify.notify_waiters();

    let mut interval = time::interval(Duration::from_secs(12 * 60 * 60));

    interval.tick().await;

    loop {
        interval.tick().await;

        log::info!("Checking for Cyrus updates (12-hour interval)...");

        match download_latest_cyrus(executor.clone()).await {
            Ok(path) => {
                log::info!("Cyrus update check completed: {:?}", path);
            }

            Err(e) => {
                log::error!("Cyrus update failed: {}", e);

                let mut lock = executor.lock().await;
                lock.download_error = Some(e);
            }
        }
    }
}

fn find_cyrus_binary(dir: &PathBuf) -> Result<PathBuf, String> {
    for entry in fs::read_dir(dir)
        .map_err(|e| format!("Failed to read directory {}: {}", dir.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;

        let path = entry.path();

        if path.is_dir() {
            if let Ok(found) = find_cyrus_binary(&path) {
                return Ok(found);
            }
        } else if path.is_file() {
            if let Some(name) = path.file_name() {
                let name_str = name.to_string_lossy();

                if (name_str == "cyrus" || name_str == "Cyrus")
                    && !name_str.ends_with(".zip")
                    && !name_str.ends_with(".sh")
                {
                    log::info!("Found Cyrus binary: {:?}", path);
                    return Ok(path);
                }
            }
        }
    }

    Err(format!("Cyrus binary not found in {}", dir.display()))
}

fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| {
        format!(
            "Failed to create destination directory {}: {}",
            dst.display(),
            e
        )
    })?;

    for entry in fs::read_dir(src)
        .map_err(|e| format!("Failed to read source directory {}: {}", src.display(), e))?
    {
        let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;

        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| {
                format!(
                    "Failed to copy {} to {}: {}",
                    src_path.display(),
                    dst_path.display(),
                    e
                )
            })?;
        }
    }

    Ok(())
}
