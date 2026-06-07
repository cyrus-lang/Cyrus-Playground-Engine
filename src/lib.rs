use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time;

pub struct CyrusExecutor {
    pub cyrus_binary_path: Option<PathBuf>,
    pub last_run_id: Option<u64>,
}

impl CyrusExecutor {
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
    executor: Arc<Mutex<CyrusExecutor>>,
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
        .map_err(|e| format!("Failed to create temp file: {}", e)).map_err(|e| e.to_string())?;

    temp_file
        .as_file()
        .write_all(code.as_bytes())
        .map_err(|e| format!("Failed to write code: {}", e)).map_err(|e| e.to_string())?;

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
        .map_err(|e| format!("Failed to execute: {}", e)).map_err(|e| e.to_string())?;

    let elapsed = start.elapsed();

    Ok(ExecutionResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        execution_time: elapsed.as_secs_f64(),
    })
}

pub async fn download_latest_cyrus(
    executor: Arc<Mutex<CyrusExecutor>>,
) -> Result<PathBuf, String> {
    let extract_dir = std::env::current_dir()
        .map_err(|e| e.to_string())?
        .join("cyrus_bin");
    
    if extract_dir.exists() {
        if let Ok(existing_binary) = find_cyrus_binary(&extract_dir) {
            let state_lock = executor.lock().await;
            if state_lock.cyrus_binary_path.is_none() {
                drop(state_lock);
                log::info!("Found existing binary, using it");
                
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let mut perms = fs::metadata(&existing_binary)
                        .map_err(|e| e.to_string())?
                        .permissions();
                    perms.set_mode(0o755);
                    fs::set_permissions(&existing_binary, perms)
                        .map_err(|e| e.to_string()).map_err(|e| e.to_string())?;
                }
                
                let mut state_lock = executor.lock().await;
                state_lock.cyrus_binary_path = Some(existing_binary.clone());
                log::info!("Binary ready (from cache)");
                return Ok(existing_binary);
            }
            drop(state_lock);
        }
    }

    let client = reqwest::Client::builder()
        .user_agent("cyrus-playground")
        .build()
        .map_err(|e| e.to_string()).map_err(|e| e.to_string())?;

    let runs_url =
        "https://api.github.com/repos/cyrus-lang/Cyrus/actions/runs?status=success&per_page=1";
    let runs_response = client.get(runs_url).send().await.map_err(|e| e.to_string()).map_err(|e| e.to_string())?;
    let runs_json: serde_json::Value = runs_response.json().await.map_err(|e| e.to_string()).map_err(|e| e.to_string())?;

    let run_id = runs_json["workflow_runs"][0]["id"]
        .as_u64()
        .ok_or("No successful runs found").map_err(|e| e.to_string())?;

    {
        let executor_lock = executor.lock().await;
        if let Some(last_id) = executor_lock.last_run_id {
            if last_id == run_id {
                if let Some(path) = &executor_lock.cyrus_binary_path {
                    if path.exists() {
                        log::info!("Binary is up to date");
                        return Ok(path.clone());
                    }
                }
            }
        }
    }

    log::info!("Downloading binary for run_id: {}", run_id);

    let artifacts_url = format!(
        "https://api.github.com/repos/cyrus-lang/Cyrus/actions/runs/{}/artifacts",
        run_id
    );
    let artifacts_response = client.get(&artifacts_url).send().await.map_err(|e| e.to_string()).map_err(|e| e.to_string())?;
    let artifacts_json: serde_json::Value = artifacts_response.json().await.map_err(|e| e.to_string()).map_err(|e| e.to_string())?;

    let artifact = artifacts_json["artifacts"]
        .as_array()
        .and_then(|arr| {
            arr.iter().find(|a| {
                a["name"].as_str().map_or(false, |name| {
                    name.to_lowercase().contains("linux") || name.to_lowercase().contains("cyrus")
                })
            })
        })
        .or_else(|| {
            artifacts_json["artifacts"]
                .as_array()
                .and_then(|arr| arr.first())
        })
        .ok_or("No artifacts found").map_err(|e| e.to_string())?;

    let artifact_name = artifact["name"].as_str().unwrap_or("artifact");

    let nightly_link_url = format!(
        "https://nightly.link/cyrus-lang/Cyrus/actions/runs/{}/{}.zip",
        run_id, artifact_name
    );

    log::info!("Downloading from nightly.link");

    let artifact_response = client.get(&nightly_link_url).send().await.map_err(|e| e.to_string())?;

    if !artifact_response.status().is_success() {
        return Err(format!("Download failed: {}", artifact_response.status()));
    }

    let bytes = artifact_response.bytes().await.map_err(|e| e.to_string())?;

    let temp_zip = tempfile::NamedTempFile::new().map_err(|e| e.to_string())?;
    fs::write(temp_zip.path(), bytes).map_err(|e| e.to_string())?;

    let file = fs::File::open(temp_zip.path()).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    let extract_dir = std::env::current_dir().map_err(|e| e.to_string())?.join("cyrus_bin");
    if extract_dir.exists() {
        fs::remove_dir_all(&extract_dir).map_err(|e| e.to_string())?;
    }
    fs::create_dir_all(&extract_dir).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        let outpath = extract_dir.join(file.name());

        if file.name().ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }

    for entry in fs::read_dir(&extract_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip") {
            log::info!("Found nested zip: {:?}", path);
            let nested_file = fs::File::open(&path).map_err(|e| e.to_string())?;
            let mut nested_archive = zip::ZipArchive::new(nested_file).map_err(|e| e.to_string())?;
            
            for i in 0..nested_archive.len() {
                let mut file = nested_archive.by_index(i).map_err(|e| e.to_string())?;
                let outpath = extract_dir.join(file.name());

                if file.name().ends_with('/') {
                    fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
                } else {
                    if let Some(parent) = outpath.parent() {
                        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
                    }
                    let mut outfile = fs::File::create(&outpath).map_err(|e| e.to_string())?;
                    std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
                }
            }
            
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }

    let binary_path = find_cyrus_binary(&extract_dir).map_err(|e| e.to_string())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&binary_path).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&binary_path, perms).map_err(|e| e.to_string())?;
    }

    let mut executor_lock = executor.lock().await;
    executor_lock.cyrus_binary_path = Some(binary_path.clone());
    executor_lock.last_run_id = Some(run_id);

    log::info!("Binary ready");

    Ok(binary_path)
}

pub async fn auto_update_cyrus(executor: Arc<Mutex<CyrusExecutor>>) {
    log::info!("Starting auto-update task");
    
    let binary_exists = {
        let lock = executor.lock().await;
        lock.cyrus_binary_path.as_ref().map(|p| p.exists()).unwrap_or(false)
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
                if (name_str == "cyrus" || name_str == "Cyrus") && !name_str.ends_with(".zip") && !name_str.ends_with(".sh") {
                    log::info!("Found Cyrus binary: {:?}", path);
                    return Ok(path);
                }
            }
        }
    }
    Err("Cyrus binary not found".to_string())
}
