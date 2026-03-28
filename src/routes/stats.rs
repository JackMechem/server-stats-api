use axum::response::Json;
use std::collections::HashMap;
use sysinfo::{Components, Disks, Networks, System};
use tokio::process::Command;

use crate::models;

pub async fn get_stats() -> Json<models::SystemStats> {
    let mut sys = System::new_all();
    sys.refresh_all();

    let memory = models::MemoryStats {
        total: sys.total_memory() / 1_000_000,
        used: sys.used_memory() / 1_000_000,
        available: sys.available_memory() / 1_000_000,
        percent: (sys.used_memory() as f64 / sys.total_memory() as f64 * 100.0) as u64,
    };

    let cpu = models::CpuStats {
        percent: sys.global_cpu_usage(),
        model: sys.cpus()[0].brand().to_string(),
        cores: sys.cpus().len(),
    };

    let disks = Disks::new_with_refreshed_list();
    let root_disk = disks
        .iter()
        .find(|d| d.mount_point() == std::path::Path::new("/"));
    let disk = if let Some(d) = root_disk {
        let total = d.total_space();
        let available = d.available_space();
        let used = total - available;
        let mb = 1024 * 1024;
        models::DiskStats {
            total: total / mb,
            used: used / mb,
            available: available / mb,
            percent: (used as f64 / total as f64 * 100.0) as u64,
        }
    } else {
        models::DiskStats {
            total: 0,
            used: 0,
            available: 0,
            percent: 0,
        }
    };

    let seconds = System::uptime();
    let uptime = models::UptimeStats {
        seconds,
        days: seconds / 86400,
        hours: (seconds % 86400) / 3600,
        minutes: (seconds % 3600) / 60,
    };

    let networks = Networks::new_with_refreshed_list();
    let network: HashMap<String, models::NetworkStats> = networks
        .iter()
        .map(|(name, data): (&String, &sysinfo::NetworkData)| {
            (
                name.clone(),
                models::NetworkStats {
                    rx: data.total_received(),
                    tx: data.total_transmitted(),
                },
            )
        })
        .collect();

    let load = System::load_average();
    let load_avg = models::LoadAvgStats {
        one: load.one,
        five: load.five,
        fifteen: load.fifteen,
    };

    let components = Components::new_with_refreshed_list();
    let temperature: f32 = components
        .iter()
        .next()
        .and_then(|c: &sysinfo::Component| c.temperature())
        .unwrap_or(0.0f32);

    let mut services: HashMap<String, String> = HashMap::new();
    for name in crate::config::ALLOWED_SERVICES {
        let output = Command::new("/run/current-system/sw/bin/systemctl")
            .args(["is-active", name])
            .output()
            .await;
        let status = match output {
            Ok(out) => String::from_utf8_lossy(&out.stdout).trim().to_string(),
            Err(_) => "unknown".to_string(),
        };
        services.insert(name.to_string(), status);
    }

    Json(models::SystemStats {
        timestamp: chrono::Utc::now().to_rfc3339(),
        memory,
        cpu,
        disk,
        uptime,
        network,
        load_avg,
        temperature,
        services,
    })
}
