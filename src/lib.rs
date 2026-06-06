mod api;
mod binding;
#[doc(hidden)]
pub mod macros;
mod module;

#[macro_use]
extern crate log;
#[cfg(target_os = "android")]
extern crate android_logger;

#[cfg(target_os = "android")]
use android_logger::Config;
#[cfg(target_os = "android")]
use log::LevelFilter;

use std::ffi::{CStr, CString};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::sync::OnceLock;

pub use api::ZygiskApi;
pub use binding::{AppSpecializeArgs, ServerSpecializeArgs, StateFlags, ZygiskOption, API_VERSION};
use jni::{JNIEnv, JavaVM};
pub use module::ZygiskModule;

const CONFIG_PATH: &str = "/data/adb/modules/AndroidUse/.config/androiduse/zygisk-target";
const REGISTRY_DIR: &str = "/data/adb/modules/AndroidUse/.config/androiduse/auzm.d";
const FALLBACK_PAYLOAD_PATH: &str = "/data/adb/modules/AndroidUse/.config/androiduse/payload.so";
const FALLBACK_MODULE_ID: &str = "androiduse-runtime";

static MODULE: ZygiskLoaderModule = ZygiskLoaderModule {};
crate::zygisk_module!(&MODULE);

struct ZygiskLoaderModule {}

static JAVA_VM: OnceLock<JavaVM> = OnceLock::new();
static PAYLOAD_BUFFERS: OnceLock<Vec<PayloadBuffer>> = OnceLock::new();
static TARGET_APP_DETECTED: OnceLock<bool> = OnceLock::new();

#[derive(Clone, Debug)]
struct AuzmEntry {
    id: String,
    path: String,
    scope: String,
}

#[derive(Debug)]
struct PayloadBuffer {
    id: String,
    data: Vec<u8>,
}

fn rand_int() -> u32 {
    // Simple pseudo-random for filename obfuscation using time
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .subsec_nanos()
}

fn write_file(path: &str, data: &[u8]) -> std::io::Result<()> {
    let mut f = File::create(path)?;
    f.write_all(data)?;
    f.sync_all()?;
    let permissions = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, permissions)?;
    Ok(())
}

fn read_file_to_memory(path: &str) -> std::io::Result<Vec<u8>> {
    let mut f = File::open(path)?;
    let mut buffer = Vec::new();
    f.read_to_end(&mut buffer)?;
    Ok(buffer)
}

impl ZygiskModule for ZygiskLoaderModule {
    fn on_load(&self, _api: ZygiskApi, env: &mut JNIEnv) {
        #[cfg(target_os = "android")]
        android_logger::init_once(
            Config::default()
                .with_max_level(LevelFilter::Debug)
                .with_tag("AndroidUseZygisk"),
        );

        let vm = env.get_java_vm().expect("Failed to get JavaVM");
        let _ = JAVA_VM.set(vm);
        info!("AndroidUse Zygisk loader initialized");
    }

    fn pre_app_specialize(&self, _api: ZygiskApi, args: &mut AppSpecializeArgs) {
        let current_process = get_process_name_from_args_safe(args);
        let entries = read_auzm_registry();
        info!(
            "Checking AUZM registry entries={} current='{}'",
            entries.len(),
            current_process
        );

        let mut buffers = Vec::new();
        for entry in entries {
            if !scope_matches(&entry.scope, &current_process) {
                continue;
            }
            match read_file_to_memory(&entry.path) {
                Ok(data) => {
                    info!(
                        "AUZM '{}' buffered to RAM from {}: {} bytes",
                        entry.id,
                        entry.path,
                        data.len()
                    );
                    buffers.push(PayloadBuffer { id: entry.id, data });
                }
                Err(err) => {
                    error!(
                        "Failed to buffer AUZM '{}' from {}: {}",
                        entry.id, entry.path, err
                    );
                }
            }
        }

        if !buffers.is_empty() {
            info!(
                "Target Detected: {} AUZM module(s) match {}",
                buffers.len(),
                current_process
            );
            let _ = TARGET_APP_DETECTED.set(true);
            let _ = PAYLOAD_BUFFERS.set(buffers);
        }
    }

    fn post_app_specialize(&self, _api: ZygiskApi, args: &AppSpecializeArgs) {
        if TARGET_APP_DETECTED.get() != Some(&true) {
            return;
        }

        if let Some(buffers) = PAYLOAD_BUFFERS.get() {
            // FIX: Use app_data_dir directly instead of nice_name
            // This ensures we write to the correct folder even for isolated processes (e.g., :remote)
            let data_dir = get_app_data_dir_from_args(args);

            if data_dir.is_empty() {
                error!("Could not determine app data directory");
                return;
            }
            let process_name = get_process_name_from_args_safe(args);
            set_payload_env("ANDROIDUSE_PROCESS_NAME", &process_name);
            set_payload_env("ANDROIDUSE_APP_DATA_DIR", &data_dir);

            for payload in buffers {
                load_payload_from_memory(payload, &data_dir);
            }
        }
    }
}

fn load_payload_from_memory(payload: &PayloadBuffer, data_dir: &str) {
    // Generate a random filename to avoid collisions and look like a cache file.
    let file_name = format!(
        "{}/cache/.androiduse_{}_{}.so",
        data_dir,
        safe_file_label(&payload.id),
        rand_int()
    );

    info!(
        "Attempting AUZM '{}' injection to: {}",
        payload.id, file_name
    );

    match write_file(&file_name, &payload.data) {
        Ok(_) => {
            let c_path = match CString::new(file_name.clone()) {
                Ok(path) => path,
                Err(err) => {
                    error!("Invalid cache path for AUZM '{}': {}", payload.id, err);
                    let _ = fs::remove_file(&file_name);
                    return;
                }
            };
            unsafe {
                let handle = libc::dlopen(c_path.as_ptr(), libc::RTLD_NOW);

                // The kernel keeps the mapping alive after unlink while the handle remains loaded.
                let _ = fs::remove_file(&file_name);

                if handle.is_null() {
                    let err = CStr::from_ptr(libc::dlerror()).to_string_lossy();
                    error!("AUZM '{}' injection failed: {}", payload.id, err);
                } else {
                    info!(
                        "AUZM '{}' injection success! Handle: {:p}",
                        payload.id, handle
                    );
                }
            }
        }
        Err(err) => error!("Failed to write AUZM '{}': {}", payload.id, err),
    }
}

fn read_auzm_registry() -> Vec<AuzmEntry> {
    let mut entries = Vec::new();
    if let Ok(dir) = fs::read_dir(REGISTRY_DIR) {
        for item in dir.flatten() {
            let path = item.path();
            if !path.is_dir() {
                continue;
            }
            let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !is_enabled(&path) {
                continue;
            }
            let payload_path = read_trimmed(path.join("path"))
                .or_else(|| read_trimmed(path.join("payload")))
                .unwrap_or_else(|| format!("{REGISTRY_DIR}/{id}/payload.so"));
            let scope = read_trimmed(path.join("scope")).unwrap_or_default();
            if payload_path.is_empty() || scope.is_empty() {
                continue;
            }
            entries.push(AuzmEntry {
                id: id.to_owned(),
                path: payload_path,
                scope,
            });
        }
    }
    if entries.is_empty() {
        entries.extend(read_fallback_entry());
    }
    entries
}

fn read_fallback_entry() -> Option<AuzmEntry> {
    let scope = read_target_config()
        .ok()
        .filter(|value| !value.is_empty())?;
    Some(AuzmEntry {
        id: FALLBACK_MODULE_ID.to_owned(),
        path: FALLBACK_PAYLOAD_PATH.to_owned(),
        scope,
    })
}

fn is_enabled(dir: &std::path::Path) -> bool {
    let value = read_trimmed(dir.join("enabled")).unwrap_or_else(|| "1".to_owned());
    matches!(
        value.as_str(),
        "" | "1" | "true" | "yes" | "on" | "enabled" | "active"
    )
}

fn read_trimmed(path: impl AsRef<std::path::Path>) -> Option<String> {
    fs::read_to_string(path)
        .ok()
        .map(|value| value.trim().to_owned())
}

fn scope_matches(scope: &str, process: &str) -> bool {
    scope
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .any(|line| line == "*" || process.contains(line))
}

fn safe_file_label(value: &str) -> String {
    let mut label = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
            label.push(ch);
        }
    }
    if label.is_empty() {
        "module".to_owned()
    } else {
        label
    }
}

fn read_target_config() -> std::io::Result<String> {
    let f = File::open(CONFIG_PATH)?;
    let mut reader = BufReader::new(f);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    Ok(line.trim().to_string())
}

// ARGS PARSING HELPERS

fn get_process_name_from_args_safe(args: &AppSpecializeArgs) -> String {
    if let Some(vm) = JAVA_VM.get() {
        // Fast-Path: Thread already attached in Zygote child process
        if let Ok(mut env) = vm.get_env() {
            if let Ok(s) = env.get_string(args.nice_name) {
                let s_rust: String = s.into();
                if !s_rust.is_empty() {
                    return s_rust;
                }
            }
        }
    }
    if let Some(cmdline) = read_proc_cmdline() {
        return cmdline;
    }
    let dir = get_app_data_dir_from_args(args);
    if !dir.is_empty() {
        return extract_package_from_path(&dir);
    }
    String::new()
}

fn read_proc_cmdline() -> Option<String> {
    let mut value = std::fs::read_to_string("/proc/self/cmdline").ok()?;
    if let Some(index) = value.find('\0') {
        value.truncate(index);
    }
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn set_payload_env(key: &str, value: &str) {
    let key = match CString::new(key) {
        Ok(key) => key,
        Err(_) => return,
    };
    let value = match CString::new(value) {
        Ok(value) => value,
        Err(_) => return,
    };
    unsafe {
        libc::setenv(key.as_ptr(), value.as_ptr(), 1);
    }
}

fn get_app_data_dir_from_args(args: &AppSpecializeArgs) -> String {
    if let Some(vm) = JAVA_VM.get() {
        // Fast-Path: Thread already attached in Zygote child process
        if let Ok(mut env) = vm.get_env() {
            if let Ok(j_str) = env.get_string(args.app_data_dir) {
                return j_str.into();
            }
        }
    }
    String::new()
}

fn extract_package_from_path(path: &str) -> String {
    let parts: Vec<&str> = path.split('/').collect();
    if parts.len() >= 3 {
        for part in parts.iter().rev() {
            if !part.is_empty() && *part != "cache" {
                return part.to_string();
            }
        }
    }
    String::new()
}

#[cfg(test)]
mod test {
    use std::os::unix::io::RawFd;
    fn companion(_socket: RawFd) {}
    crate::zygisk_companion!(companion);
}
