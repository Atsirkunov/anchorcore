//! R45.6: machine facts for the onboarding wizard's local answer-model
//! picker — total RAM and an Ollama answer model sized to it. Read-only,
//! no network; `None` when the platform won't say.

use serde_json::{json, Value};

const GIB: f64 = 1024.0 * 1024.0 * 1024.0;

/// One answer model the picker offers. `min_ram_gb` is the machine RAM at
/// which the model still leaves room for the OS, the app and the index.
pub struct AnswerModel {
    pub model: &'static str,
    pub label: &'static str,
    pub download_gb: f64,
    pub min_ram_gb: f64,
}

/// Smallest first. Ollama default (q4) download sizes.
pub const ANSWER_MODELS: &[AnswerModel] = &[
    AnswerModel { model: "llama3.2:1b", label: "Llama 3.2 1B — fastest, basic answers", download_gb: 1.3, min_ram_gb: 0.0 },
    AnswerModel { model: "llama3.2:3b", label: "Llama 3.2 3B — small and quick", download_gb: 2.0, min_ram_gb: 8.0 },
    AnswerModel { model: "qwen2.5:7b", label: "Qwen 2.5 7B — good answers", download_gb: 4.7, min_ram_gb: 16.0 },
    AnswerModel { model: "qwen2.5:14b", label: "Qwen 2.5 14B — better answers", download_gb: 9.0, min_ram_gb: 32.0 },
    AnswerModel { model: "qwen2.5:32b", label: "Qwen 2.5 32B — best answers, slower", download_gb: 20.0, min_ram_gb: 64.0 },
];

/// Used when the RAM is unknown: the long-standing wizard default.
pub const FALLBACK_ANSWER_MODEL: &str = "llama3.2:3b";

/// The largest model whose RAM floor the machine meets. Half a GiB of
/// slack, because firmware reserves some memory (a "16 GB" machine can
/// report 15.6 GiB).
pub fn recommended_answer_model(total_bytes: Option<u64>) -> &'static str {
    let Some(bytes) = total_bytes else { return FALLBACK_ANSWER_MODEL };
    let gib = bytes as f64 / GIB + 0.5;
    ANSWER_MODELS
        .iter()
        .rev()
        .find(|m| m.min_ram_gb <= gib)
        .map_or(ANSWER_MODELS[0].model, |m| m.model)
}

/// `machine` block of `GET /system/onboarding`.
pub fn machine_json() -> Value {
    let total = total_memory_bytes();
    let recommended = recommended_answer_model(total);
    let models: Vec<Value> = ANSWER_MODELS
        .iter()
        .map(|m| {
            json!({
                "model": m.model,
                "label": m.label,
                "download_gb": m.download_gb,
                "min_ram_gb": m.min_ram_gb,
                "recommended": m.model == recommended,
            })
        })
        .collect();
    json!({
        "total_memory_bytes": total,
        "recommended_answer_model": recommended,
        "answer_models": models,
    })
}

/// Physical RAM in bytes (on Linux capped by a cgroup v2 memory limit, so a
/// container reports what it may actually use).
pub fn total_memory_bytes() -> Option<u64> {
    imp::total_memory_bytes().filter(|b| *b > 0)
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn total_memory_bytes() -> Option<u64> {
        let mut value: u64 = 0;
        let mut len = std::mem::size_of::<u64>();
        let name = c"hw.memsize";
        // SAFETY: `hw.memsize` is a u64 sysctl; buffer and length match it.
        let rc = unsafe {
            libc::sysctlbyname(name.as_ptr(), (&mut value as *mut u64).cast(), &mut len, std::ptr::null_mut(), 0)
        };
        (rc == 0 && len == std::mem::size_of::<u64>()).then_some(value)
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn total_memory_bytes() -> Option<u64> {
        let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
        let total = super::parse_meminfo_total(&meminfo)?;
        let limit = std::fs::read_to_string("/sys/fs/cgroup/memory.max")
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok());
        Some(limit.map_or(total, |l| l.min(total)))
    }
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

    pub fn total_memory_bytes() -> Option<u64> {
        // SAFETY: a zeroed MEMORYSTATUSEX with dwLength set is the documented input.
        let mut status: MEMORYSTATUSEX = unsafe { std::mem::zeroed() };
        status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let ok = unsafe { GlobalMemoryStatusEx(&mut status) };
        (ok != 0).then_some(status.ullTotalPhys)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
    pub fn total_memory_bytes() -> Option<u64> {
        None
    }
}

/// `MemTotal:       16314824 kB` → bytes.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_meminfo_total(meminfo: &str) -> Option<u64> {
    let line = meminfo.lines().find(|l| l.starts_with("MemTotal:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    const G: u64 = 1024 * 1024 * 1024;

    #[test]
    fn recommendation_follows_ram_tiers() {
        assert_eq!(recommended_answer_model(None), FALLBACK_ANSWER_MODEL);
        assert_eq!(recommended_answer_model(Some(4 * G)), "llama3.2:1b");
        assert_eq!(recommended_answer_model(Some(8 * G)), "llama3.2:3b");
        assert_eq!(recommended_answer_model(Some(16 * G)), "qwen2.5:7b");
        assert_eq!(recommended_answer_model(Some(24 * G)), "qwen2.5:7b");
        assert_eq!(recommended_answer_model(Some(32 * G)), "qwen2.5:14b");
        assert_eq!(recommended_answer_model(Some(128 * G)), "qwen2.5:32b");
    }

    #[test]
    fn firmware_reserved_memory_keeps_the_tier() {
        // A "16 GB" machine that reports 15.6 GiB.
        assert_eq!(recommended_answer_model(Some(15 * G + 600 * 1024 * 1024)), "qwen2.5:7b");
    }

    #[test]
    fn machine_json_marks_exactly_one_recommended() {
        let v = machine_json();
        let models = v["answer_models"].as_array().unwrap();
        assert_eq!(models.len(), ANSWER_MODELS.len());
        let rec: Vec<&Value> = models.iter().filter(|m| m["recommended"] == true).collect();
        assert_eq!(rec.len(), 1);
        assert_eq!(rec[0]["model"], v["recommended_answer_model"]);
    }

    #[test]
    fn meminfo_total_parses_kb() {
        let text = "MemTotal:       16314824 kB\nMemFree:  1 kB\n";
        assert_eq!(parse_meminfo_total(text), Some(16314824 * 1024));
        assert_eq!(parse_meminfo_total("garbage"), None);
    }

    #[test]
    fn this_machine_reports_ram() {
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(total_memory_bytes().unwrap_or(0) > G / 2);
        }
    }
}
