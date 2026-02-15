use anyhow::{Context, Result};
use once_cell::sync::OnceCell;
use regex::Regex;
use reqwest::blocking::Client;
use serde_json::Value;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use zip::ZipArchive;

const VERSION_API: &str = "https://googlechromelabs.github.io/chrome-for-testing/last-known-good-versions-with-downloads.json";
const PATCH_API: &str = "https://googlechromelabs.github.io/chrome-for-testing/latest-patch-versions-per-build-with-downloads.json";

#[derive(Debug)]
pub struct ChromeDriverManager {
    cache_dir: PathBuf,
    client: Client,
}

impl ChromeDriverManager {
    pub fn new(cache_dir: Option<PathBuf>) -> Result<Self> {
        let dir = cache_dir.unwrap_or_else(default_cache_dir);
        fs::create_dir_all(&dir).with_context(|| format!("创建缓存目录失败: {}", dir.display()))?;
        let client = Client::builder()
            .timeout(std::time::Duration::from_secs(20))
            .build()
            .with_context(|| "创建 HTTP 客户端失败")?;
        Ok(Self {
            cache_dir: dir,
            client,
        })
    }

    pub fn get_chrome_version(&self) -> Option<String> {
        #[cfg(windows)]
        {
            if let Some(version) = self.get_chrome_version_from_windows_registry() {
                tracing::info!("检测到Chrome版本(注册表): {}", version);
                return Some(version);
            }
        }

        for cmd in chrome_version_commands() {
            let mut c = Command::new(&cmd.0);
            c.args(&cmd.1);
            #[cfg(windows)]
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            let output = match c.output() {
                Ok(item) => item,
                Err(_) => continue,
            };
            if !output.status.success() {
                continue;
            }
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(version) = parse_chrome_version(&text) {
                tracing::info!("检测到Chrome版本: {}", version);
                return Some(version);
            }

            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Some(version) = parse_chrome_version(&stderr) {
                tracing::info!("检测到Chrome版本: {}", version);
                return Some(version);
            }
        }
        tracing::warn!("未能检测到Chrome版本");
        None
    }

    pub fn get_major_version(version: &str) -> u32 {
        version
            .split('.')
            .next()
            .and_then(|item| item.parse::<u32>().ok())
            .unwrap_or(0)
    }

    pub fn find_cached_driver(&self, version: &str) -> Option<PathBuf> {
        let driver_name = driver_binary_name();
        let exact = self
            .cache_dir
            .join(format!("chromedriver_{version}"))
            .join(driver_name);
        if exact.exists() {
            tracing::info!("使用缓存的ChromeDriver: {}", exact.display());
            return Some(exact);
        }

        let major = Self::get_major_version(version);
        let prefix = format!("chromedriver_{major}.");
        let entries = fs::read_dir(&self.cache_dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name()?.to_string_lossy();
            if !file_name.starts_with(&prefix) {
                continue;
            }
            let candidate = find_file_recursively(&path, driver_name);
            if let Some(item) = candidate {
                tracing::info!("使用主版本缓存的ChromeDriver: {}", item.display());
                return Some(item);
            }
        }
        None
    }

    pub fn get_or_download_driver(&self, chrome_version: Option<&str>) -> Result<PathBuf> {
        if let Some(item) = chrome_version {
            let version = item.trim();
            if !version.is_empty() {
                return self.get_or_download_driver_by_version(version);
            }
        }

        if let Some(version) = self.get_chrome_version() {
            return self.get_or_download_driver_by_version(&version);
        }

        if let Some(found) = self.find_any_cached_driver() {
            tracing::warn!("未检测到Chrome版本，使用任意缓存驱动: {}", found.display());
            return Ok(found);
        }

        let guard = download_lock()
            .lock()
            .map_err(|_| anyhow::anyhow!("驱动下载锁获取失败"))?;
        let _guard = guard;
        if let Some(found) = self.find_any_cached_driver() {
            tracing::warn!(
                "未检测到Chrome版本，其他线程已准备可用缓存驱动: {}",
                found.display()
            );
            return Ok(found);
        }
        tracing::warn!("未检测到Chrome版本，尝试下载最新稳定版ChromeDriver");
        self.download_latest_stable_driver()
    }

    pub fn get_driver_path(&self) -> Result<PathBuf> {
        if let Ok(path) = std::env::var("CHROMEDRIVER_PATH") {
            let item = PathBuf::from(path.trim());
            if item.exists() {
                return Ok(item);
            }
        }

        let local = PathBuf::from(driver_binary_name());
        if local.exists() {
            return Ok(local);
        }

        self.get_or_download_driver(None)
    }

    fn download_driver(&self, chrome_version: &str) -> Result<PathBuf> {
        let platform = platform_name();
        let url = self
            .get_driver_url(chrome_version, &platform)
            .with_context(|| format!("未找到匹配的ChromeDriver下载地址: version={chrome_version}, platform={platform}"))?;
        self.download_driver_from_url(chrome_version, &url)
    }

    fn get_driver_url(&self, chrome_version: &str, platform: &str) -> Option<String> {
        if let Some(url) = self.get_direct_url(chrome_version, platform) {
            return Some(url);
        }
        if let Some(url) = self.get_patch_url(chrome_version, platform) {
            return Some(url);
        }
        self.get_last_known_url(platform, chrome_version)
    }

    fn get_direct_url(&self, chrome_version: &str, platform: &str) -> Option<String> {
        let url = format!(
            "https://storage.googleapis.com/chrome-for-testing-public/{}/{}/chromedriver-{}.zip",
            chrome_version, platform, platform
        );
        let ok = self
            .client
            .head(&url)
            .send()
            .map(|item| item.status().is_success())
            .unwrap_or(false);
        if ok { Some(url) } else { None }
    }

    fn get_patch_url(&self, chrome_version: &str, platform: &str) -> Option<String> {
        let build_prefix = chrome_version
            .split('.')
            .take(3)
            .collect::<Vec<_>>()
            .join(".");
        let payload: Value = self.client.get(PATCH_API).send().ok()?.json().ok()?;
        let build = payload.get("builds")?.get(&build_prefix)?;
        let items = build.get("downloads")?.get("chromedriver")?.as_array()?;
        for item in items {
            let p = item
                .get("platform")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if p == platform {
                let url = item.get("url").and_then(Value::as_str)?.to_string();
                return Some(url);
            }
        }
        None
    }

    fn get_last_known_url(&self, platform: &str, chrome_version: &str) -> Option<String> {
        let major = Self::get_major_version(chrome_version);
        let payload: Value = self.client.get(VERSION_API).send().ok()?.json().ok()?;
        let channels = payload.get("channels")?.as_object()?;
        for (_name, channel) in channels {
            let version = channel
                .get("version")
                .and_then(Value::as_str)
                .unwrap_or_default();
            if Self::get_major_version(version) != major {
                continue;
            }
            let downloads = channel.get("downloads")?.get("chromedriver")?.as_array()?;
            for item in downloads {
                let p = item
                    .get("platform")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if p == platform {
                    return item
                        .get("url")
                        .and_then(Value::as_str)
                        .map(|s| s.to_string());
                }
            }
        }
        None
    }

    fn get_or_download_driver_by_version(&self, chrome_version: &str) -> Result<PathBuf> {
        if let Some(found) = self.find_cached_driver(chrome_version) {
            return Ok(found);
        }

        let guard = download_lock()
            .lock()
            .map_err(|_| anyhow::anyhow!("驱动下载锁获取失败"))?;
        let _guard = guard;
        if let Some(found) = self.find_cached_driver(chrome_version) {
            return Ok(found);
        }
        tracing::info!("未找到缓存驱动，开始下载");
        self.download_driver(chrome_version)
    }

    fn find_any_cached_driver(&self) -> Option<PathBuf> {
        let driver_name = driver_binary_name();
        let entries = fs::read_dir(&self.cache_dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|item| item.to_str()) else {
                continue;
            };
            if !name.starts_with("chromedriver_") {
                continue;
            }
            if let Some(found) = find_file_recursively(&path, driver_name) {
                return Some(found);
            }
        }
        None
    }

    fn download_latest_stable_driver(&self) -> Result<PathBuf> {
        let platform = platform_name();
        let payload: Value = self
            .client
            .get(VERSION_API)
            .send()
            .and_then(|item| item.error_for_status())
            .with_context(|| "请求Chrome for Testing版本信息失败")?
            .json()
            .with_context(|| "解析Chrome for Testing版本信息失败")?;
        let channels = payload
            .get("channels")
            .and_then(Value::as_object)
            .with_context(|| "版本信息缺少channels字段")?;

        for channel_name in ["Stable", "Beta", "Dev", "Canary"] {
            let Some(channel) = channels.get(channel_name) else {
                continue;
            };
            let Some(version) = channel.get("version").and_then(Value::as_str) else {
                continue;
            };
            let downloads = channel
                .get("downloads")
                .and_then(|v| v.get("chromedriver"))
                .and_then(Value::as_array);
            let Some(items) = downloads else {
                continue;
            };
            for item in items {
                let p = item
                    .get("platform")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if p != platform {
                    continue;
                }
                let Some(url) = item.get("url").and_then(Value::as_str) else {
                    continue;
                };
                tracing::info!(
                    "使用{}通道ChromeDriver下载: version={}, platform={}",
                    channel_name,
                    version,
                    platform
                );
                return self.download_driver_from_url(version, url);
            }
        }

        anyhow::bail!("无法获取平台 {} 的最新ChromeDriver下载地址", platform)
    }

    fn download_driver_from_url(&self, version_tag: &str, url: &str) -> Result<PathBuf> {
        tracing::info!("下载ChromeDriver: {}", url);
        let response = self
            .client
            .get(url)
            .send()
            .and_then(|item| item.error_for_status())
            .with_context(|| "下载ChromeDriver失败")?;
        let data = response.bytes().with_context(|| "读取驱动压缩包失败")?;

        let temp_zip = self.cache_dir.join(format!(
            "chromedriver_{}.zip.tmp",
            chrono::Local::now().timestamp_millis()
        ));
        {
            let mut file = File::create(&temp_zip)
                .with_context(|| format!("创建临时文件失败: {}", temp_zip.display()))?;
            file.write_all(&data)
                .with_context(|| format!("写入临时压缩包失败: {}", temp_zip.display()))?;
        }

        let target_dir = self.cache_dir.join(format!("chromedriver_{version_tag}"));
        fs::create_dir_all(&target_dir)
            .with_context(|| format!("创建目标目录失败: {}", target_dir.display()))?;
        unzip_file(&temp_zip, &target_dir)?;
        let _ = fs::remove_file(&temp_zip);

        let driver = find_file_recursively(&target_dir, driver_binary_name())
            .with_context(|| "压缩包中未找到 chromedriver 可执行文件")?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&driver)
                .with_context(|| format!("读取驱动权限失败: {}", driver.display()))?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&driver, perms)
                .with_context(|| format!("设置驱动执行权限失败: {}", driver.display()))?;
        }
        tracing::info!("ChromeDriver下载完成: {}", driver.display());
        Ok(driver)
    }

    #[cfg(windows)]
    fn get_chrome_version_from_windows_registry(&self) -> Option<String> {
        let queries = [
            (r"HKCU\SOFTWARE\Google\Chrome\BLBeacon", "version"),
            (
                r"HKCU\SOFTWARE\Wow6432Node\Google\Chrome\BLBeacon",
                "version",
            ),
            (r"HKLM\SOFTWARE\Google\Chrome\BLBeacon", "version"),
            (
                r"HKLM\SOFTWARE\Wow6432Node\Google\Chrome\BLBeacon",
                "version",
            ),
        ];

        for (path, value_name) in queries {
            let mut c = Command::new("reg");
            c.args(["query", path, "/v", value_name]);
            {
                use std::os::windows::process::CommandExt;
                c.creation_flags(0x08000000); // CREATE_NO_WINDOW
            }
            let output = match c.output() {
                Ok(item) => item,
                Err(_) => continue,
            };
            if !output.status.success() {
                continue;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Some(version) = parse_chrome_version(&stdout) {
                return Some(version);
            }
            let stderr = String::from_utf8_lossy(&output.stderr);
            if let Some(version) = parse_chrome_version(&stderr) {
                return Some(version);
            }
        }
        None
    }
}

pub fn get_chromedriver_path() -> Result<PathBuf> {
    static INSTANCE: OnceCell<ChromeDriverManager> = OnceCell::new();
    let manager = INSTANCE.get_or_try_init(|| ChromeDriverManager::new(None))?;
    manager.get_driver_path()
}

fn default_cache_dir() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        return PathBuf::from(home).join(".cache").join("chromedriver");
    }
    if let Ok(profile) = std::env::var("USERPROFILE") {
        return PathBuf::from(profile).join(".cache").join("chromedriver");
    }
    PathBuf::from(".cache").join("chromedriver")
}

fn driver_binary_name() -> &'static str {
    if cfg!(windows) {
        "chromedriver.exe"
    } else {
        "chromedriver"
    }
}

fn platform_name() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match os {
        "windows" => {
            if arch.contains("64") {
                "win64".to_string()
            } else {
                "win32".to_string()
            }
        }
        "macos" => {
            if arch.contains("aarch64") || arch.contains("arm") {
                "mac-arm64".to_string()
            } else {
                "mac-x64".to_string()
            }
        }
        _ => "linux64".to_string(),
    }
}

fn chrome_version_commands() -> Vec<(String, Vec<String>)> {
    if cfg!(windows) {
        vec![
            (
                r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
                vec!["--version".to_string()],
            ),
            (
                r"C:\Program Files (x86)\Google\Chrome\Application\chrome.exe".to_string(),
                vec!["--version".to_string()],
            ),
            ("chrome".to_string(), vec!["--version".to_string()]),
        ]
    } else if cfg!(target_os = "macos") {
        vec![
            (
                "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
                vec!["--version".to_string()],
            ),
            ("google-chrome".to_string(), vec!["--version".to_string()]),
        ]
    } else {
        vec![
            ("google-chrome".to_string(), vec!["--version".to_string()]),
            (
                "google-chrome-stable".to_string(),
                vec!["--version".to_string()],
            ),
            (
                "chromium-browser".to_string(),
                vec!["--version".to_string()],
            ),
            ("chromium".to_string(), vec!["--version".to_string()]),
        ]
    }
}

fn parse_chrome_version(text: &str) -> Option<String> {
    let re = Regex::new(r"(\d+\.\d+\.\d+\.\d+)").ok()?;
    re.captures(text)
        .and_then(|cap| cap.get(1))
        .map(|m| m.as_str().to_string())
}

fn find_file_recursively(root: &Path, file_name: &str) -> Option<PathBuf> {
    if !root.exists() {
        return None;
    }
    if root.is_file() {
        let is_match = root
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(file_name))
            .unwrap_or(false);
        return if is_match {
            Some(root.to_path_buf())
        } else {
            None
        };
    }
    let entries = fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursively(&path, file_name) {
                return Some(found);
            }
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| name.eq_ignore_ascii_case(file_name))
            .unwrap_or(false)
        {
            return Some(path);
        }
    }
    None
}

fn unzip_file(zip_path: &Path, target_dir: &Path) -> Result<()> {
    let file =
        File::open(zip_path).with_context(|| format!("打开压缩包失败: {}", zip_path.display()))?;
    let mut archive =
        ZipArchive::new(file).with_context(|| format!("读取压缩包失败: {}", zip_path.display()))?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).with_context(|| "读取压缩包条目失败")?;
        let enclosed = entry
            .enclosed_name()
            .map(|p| p.to_path_buf())
            .with_context(|| "压缩包条目路径非法")?;
        let outpath = target_dir.join(enclosed);
        if entry.name().ends_with('/') {
            fs::create_dir_all(&outpath)
                .with_context(|| format!("创建目录失败: {}", outpath.display()))?;
            continue;
        }
        if let Some(parent) = outpath.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("创建父目录失败: {}", parent.display()))?;
        }
        let mut outfile = File::create(&outpath)
            .with_context(|| format!("创建输出文件失败: {}", outpath.display()))?;
        io::copy(&mut entry, &mut outfile)
            .with_context(|| format!("写入输出文件失败: {}", outpath.display()))?;
    }
    Ok(())
}

fn download_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}
