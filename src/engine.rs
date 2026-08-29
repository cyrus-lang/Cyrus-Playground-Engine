use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;

pub struct Executor {
    pub cyrus_binary_path: Option<PathBuf>,
    pub last_run_id: Option<u64>,
}

impl Executor {
    pub fn new() -> Self {
        Self {
            cyrus_binary_path: None,
            last_run_id: None,
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
    let executor_lock = executor.lock().await;
    let binary_path = match &executor_lock.cyrus_binary_path {
        Some(path) => path.clone(),
        None => {
            drop(executor_lock);
            return Err("Cyrus binary not found. Please wait for download.".to_string());
        }
    };
    drop(executor_lock);

    let temp_file = tempfile::Builder::new()
        .suffix(".cyrus")
        .tempfile()
        .map_err(|e| format!("Failed to create temp file: {}", e))
        .map_err(|e| e.to_string())?;

    temp_file
        .as_file()
        .write_all(code.as_bytes())
        .map_err(|e| format!("Failed to write code: {}", e))
        .map_err(|e| e.to_string())?;

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
        .map_err(|e| format!("Failed to execute: {}", e))
        .map_err(|e| e.to_string())?;

    let elapsed = start.elapsed();

    Ok(ExecutionResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        execution_time: elapsed.as_secs_f64(),
    })
}

pub async fn download_latest_cyrus(executor: Arc<Mutex<Executor>>) -> Result<PathBuf, String> {
    const REPO: &str = "cyrus-lang/Cyrus";
    const WORKFLOW: &str = "build-linux.yml";
    const BRANCH: &str = "main";
    const ARTIFACT_SUFFIX: &str = "-binary";

    let client = reqwest::Client::builder()
        .user_agent("cyrus-playground")
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {e}"))?;

    /*
     * IMPORTANT:
     *
     * Do NOT use:
     *
     * /actions/runs?status=success&per_page=1
     *
     * because that includes successful PR builds and runs from unrelated
     * workflows.
     *
     * Query the actual build workflow, main branch, and push events only.
     */
    let runs_url = format!(
        "https://api.github.com/repos/{REPO}/actions/workflows/{WORKFLOW}/runs\
         ?branch={BRANCH}&event=push&status=success&per_page=1"
    );

    log::info!("Looking for latest successful {WORKFLOW} build on {BRANCH}");

    let runs_response = client
        .get(&runs_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("Failed to query workflow runs: {e}"))?;

    if !runs_response.status().is_success() {
        return Err(format!(
            "Failed to query workflow runs: {}",
            runs_response.status()
        ));
    }

    let runs_json: serde_json::Value = runs_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse workflow runs response: {e}"))?;

    let run = runs_json["workflow_runs"]
        .as_array()
        .and_then(|runs| runs.first())
        .ok_or_else(|| "No successful main-branch builds found".to_string())?;

    let run_id = run["id"]
        .as_u64()
        .ok_or_else(|| "Latest workflow run has no valid ID".to_string())?;

    let commit_sha = run["head_sha"].as_str().unwrap_or("unknown");

    let run_number = run["run_number"].as_u64().unwrap_or(0);

    log::info!(
        "Latest production build: run_id={}, run_number={}, commit={}",
        run_id,
        run_number,
        commit_sha
    );

    /*
     * If we already have this exact run, there is nothing to download.
     *
     * This check happens AFTER querying GitHub so a stale local cache can
     * never prevent discovery of a newer build.
     */
    {
        let lock = executor.lock().await;

        if lock.last_run_id == Some(run_id) {
            if let Some(path) = &lock.cyrus_binary_path {
                if path.exists() {
                    log::info!("Cyrus binary is already up to date");
                    return Ok(path.clone());
                }
            }
        }
    }

    /*
     * Get artifacts belonging specifically to this workflow run.
     */
    let artifacts_url =
        format!("https://api.github.com/repos/{REPO}/actions/runs/{run_id}/artifacts");

    let artifacts_response = client
        .get(&artifacts_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("Failed to query artifacts: {e}"))?;

    if !artifacts_response.status().is_success() {
        return Err(format!(
            "Failed to query artifacts: {}",
            artifacts_response.status()
        ));
    }

    let artifacts_json: serde_json::Value = artifacts_response
        .json()
        .await
        .map_err(|e| format!("Failed to parse artifacts response: {e}"))?;

    /*
     * Select ONLY the real binary artifact.
     *
     * Do not use "contains cyrus" or "contains linux":
     * the workflow currently produces:
     *
     *   cyrus-<VERSION>-binary
     *   cyrus-<VERSION>-portable
     *   cyrus-<VERSION>-pkgbuild
     *   cyrus-<VERSION>-deb
     *   cyrus-<VERSION>-rpm
     *
     * We need the artifact containing stdlib as well as the compiler.
     */
    let artifact = artifacts_json["artifacts"]
        .as_array()
        .and_then(|artifacts| {
            artifacts.iter().find(|artifact| {
                artifact["name"]
                    .as_str()
                    .map(|name| name.ends_with(ARTIFACT_SUFFIX))
                    .unwrap_or(false)
                    && artifact["expired"]
                        .as_bool()
                        .map(|expired| !expired)
                        .unwrap_or(true)
            })
        })
        .ok_or_else(|| {
            format!(
                "No non-expired Cyrus binary artifact found for run {}",
                run_id
            )
        })?;

    let artifact_id = artifact["id"]
        .as_u64()
        .ok_or_else(|| "Artifact has no valid ID".to_string())?;

    let artifact_name = artifact["name"].as_str().unwrap_or("unknown");

    log::info!("Selected artifact: {} (id={})", artifact_name, artifact_id);

    /*
     * Download directly from GitHub.
     *
     * No nightly.link.
     */
    let artifact_url =
        format!("https://api.github.com/repos/{REPO}/actions/artifacts/{artifact_id}/zip");

    log::info!("Downloading artifact directly from GitHub");

    let artifact_response = client
        .get(&artifact_url)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|e| format!("Failed to download artifact: {e}"))?;

    if !artifact_response.status().is_success() {
        return Err(format!(
            "Artifact download failed: {}",
            artifact_response.status()
        ));
    }

    let bytes = artifact_response
        .bytes()
        .await
        .map_err(|e| format!("Failed to read artifact: {e}"))?;

    /*
     * Extract into a temporary directory first.
     *
     * This prevents a partially downloaded/broken artifact from destroying
     * the currently working compiler.
     */
    let temp_dir =
        tempfile::tempdir().map_err(|e| format!("Failed to create temporary directory: {e}"))?;

    let temp_zip = temp_dir.path().join("artifact.zip");

    fs::write(&temp_zip, &bytes).map_err(|e| format!("Failed to write artifact: {e}"))?;

    let file = fs::File::open(&temp_zip).map_err(|e| format!("Failed to open artifact: {e}"))?;

    let mut archive =
        zip::ZipArchive::new(file).map_err(|e| format!("Invalid artifact ZIP: {e}"))?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read ZIP entry: {e}"))?;

        let relative_path = PathBuf::from(file.name());

        /*
         * Protect against ZIP path traversal.
         */
        if relative_path
            .components()
            .any(|component| matches!(component, std::path::Component::ParentDir))
        {
            return Err(format!("Unsafe path in artifact: {}", file.name()));
        }

        let outpath = temp_dir.path().join(&relative_path);

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| format!("Failed to create directory: {e}"))?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create directory: {e}"))?;
            }

            let mut outfile = fs::File::create(&outpath)
                .map_err(|e| format!("Failed to create extracted file: {e}"))?;

            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to extract artifact: {e}"))?;
        }
    }

    /*
     * Replace the old installation atomically-ish:
     *
     *   cyrus_bin/
     *       cyrus
     *       stdlib/
     *
     * We only touch the real cache after the complete download/extraction
     * succeeded.
     */
    let extract_dir = std::env::current_dir()
        .map_err(|e| format!("Failed to get current directory: {e}"))?
        .join("cyrus_bin");

    let new_dir = std::env::current_dir()
        .map_err(|e| format!("Failed to get current directory: {e}"))?
        .join("cyrus_bin.new");

    if new_dir.exists() {
        fs::remove_dir_all(&new_dir)
            .map_err(|e| format!("Failed to remove old temporary installation: {e}"))?;
    }

    fs::create_dir_all(&new_dir)
        .map_err(|e| format!("Failed to create installation directory: {e}"))?;

    /*
     * Copy the extracted artifact into cyrus_bin.new.
     */
    copy_dir_recursive(temp_dir.path(), &new_dir)
        .map_err(|e| format!("Failed to install artifact: {e}"))?;

    let new_binary = find_cyrus_binary(&new_dir)
        .map_err(|e| format!("Installed artifact does not contain Cyrus: {e}"))?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let mut perms = fs::metadata(&new_binary)
            .map_err(|e| format!("Failed to stat Cyrus binary: {e}"))?
            .permissions();

        perms.set_mode(0o755);

        fs::set_permissions(&new_binary, perms)
            .map_err(|e| format!("Failed to set Cyrus permissions: {e}"))?;
    }

    /*
     * Store the run ID alongside the binary.
     *
     * This makes the local cache identifiable.
     */
    fs::write(new_dir.join(".run-id"), run_id.to_string())
        .map_err(|e| format!("Failed to write run metadata: {e}"))?;

    /*
     * Remove old installation only after the new one is valid.
     */
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir)
            .map_err(|e| format!("Failed to remove old Cyrus installation: {e}"))?;
    }

    fs::rename(&new_dir, &extract_dir)
        .map_err(|e| format!("Failed to install new Cyrus binary: {e}"))?;

    let binary_path = find_cyrus_binary(&extract_dir)
        .map_err(|e| format!("Installed Cyrus binary cannot be found: {e}"))?;

    /*
     * Update executor state.
     */
    let mut lock = executor.lock().await;

    lock.cyrus_binary_path = Some(binary_path.clone());
    lock.last_run_id = Some(run_id);

    log::info!(
        "Cyrus binary updated successfully: run={} commit={}",
        run_id,
        commit_sha
    );

    Ok(binary_path)
}

pub async fn auto_update_cyrus(executor: Arc<Mutex<Executor>>) {
    log::info!("Starting auto-update task");

    let binary_exists = {
        let lock = executor.lock().await;
        lock.cyrus_binary_path
            .as_ref()
            .map(|p| p.exists())
            .unwrap_or(false)
    };

    if !binary_exists {
        log::info!("Binary not found, downloading...");
        if let Err(e) = download_latest_cyrus(executor.clone()).await {
            log::error!("Initial download failed: {}", e);
        }
    } else {
        log::info!("Binary already exists, skipping initial download");
    }

    let mut interval = time::interval(Duration::from_secs(12 * 60 * 60));

    loop {
        interval.tick().await;
        log::info!("Checking for updates (12-hour interval)...");

        match download_latest_cyrus(executor.clone()).await {
            Ok(_) => log::info!("Update check completed"),
            Err(e) => log::error!("Update failed: {}", e),
        }
    }
}

fn find_cyrus_binary(dir: &PathBuf) -> Result<PathBuf, String> {
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
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
    Err("Cyrus binary not found".to_string())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(dst).map_err(|e| e.to_string())?;

    for entry in fs::read_dir(src).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());

        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path).map_err(|e| e.to_string())?;
        }
    }

    Ok(())
}
