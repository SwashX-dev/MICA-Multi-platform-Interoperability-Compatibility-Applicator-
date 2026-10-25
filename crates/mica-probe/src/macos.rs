//! macOS host facts, from `sw_vers`, `sysctl`, and `system_profiler`.

use serde_json::Value;

use crate::{Context, Gpu, GpuVendor, HostProfile, Init};

pub(crate) fn probe(context: &Context, profile: &mut HostProfile) {
    profile.init = Init::NotApplicable;
    let name = context.run("sw_vers", &["-productName"]);
    let version = context.run("sw_vers", &["-productVersion"]);
    profile.os_name = match (&name, &version) {
        (Some(name), Some(version)) => format!("{name} {version}"),
        _ => "macOS".into(),
    };
    profile.os_version = version;
    profile.kernel = context.run("uname", &["-r"]);
    profile.cpu_model = context.run("sysctl", &["-n", "machdep.cpu.brand_string"]);
    profile.translated = context
        .run("sysctl", &["-n", "sysctl.proc_translated"])
        .is_some_and(|value| value == "1");
    profile.virtualization.cpu_support = context
        .run("sysctl", &["-n", "kern.hv_support"])
        .map(|value| value == "1");
    let model = context.run("sysctl", &["-n", "hw.model"]);
    profile.device.vendor = Some("Apple".into());
    profile.device.product = model.clone();
    if model.as_deref().is_some_and(|model| model.starts_with("VirtualMac")) {
        profile.virtualization.guest_of = Some("apple virtualization framework".into());
    }
    profile.gpus = context
        .run("system_profiler", &["SPDisplaysDataType", "-json"])
        .and_then(|json| serde_json::from_str::<Value>(&json).ok())
        .map(|value| gpus(&value))
        .unwrap_or_default();
    profile.vulkan.loader = [
        "/opt/homebrew/lib/libvulkan.1.dylib",
        "/usr/local/lib/libvulkan.1.dylib",
    ]
    .iter()
    .any(|path| context.exists(path));
    profile.vulkan.drivers = [
        "/opt/homebrew/share/vulkan/icd.d",
        "/usr/local/share/vulkan/icd.d",
    ]
    .iter()
    .flat_map(|dir| context.list(dir))
    .filter(|name| name.ends_with(".json"))
    .collect();
}

fn gpus(value: &Value) -> Vec<Gpu> {
    value["SPDisplaysDataType"]
        .as_array()
        .map(|displays| {
            displays
                .iter()
                .filter_map(|display| {
                    let name = display["sppci_model"].as_str()?.to_string();
                    let metal = display["spdisplays_mtlgpufamilysupport"]
                        .as_str()
                        .map(|family| family.trim_start_matches("spdisplays_").to_string());
                    Some(Gpu {
                        vendor: GpuVendor::from_name(&name),
                        name,
                        driver: metal,
                        location: display["sppci_bus"].as_str().map(str::to_string),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use crate::{Os, probe_with};

    #[test]
    fn reads_sw_vers_sysctl_and_system_profiler() {
        let context = crate::Context::fake(
            &std::env::temp_dir(),
            &[],
            Box::new(|program, args| {
                let answer = match (program, args.last().copied().unwrap_or_default()) {
                    ("sw_vers", "-productName") => "macOS",
                    ("sw_vers", "-productVersion") => "26.1",
                    ("sysctl", "machdep.cpu.brand_string") => "Apple M3 Pro",
                    ("sysctl", "sysctl.proc_translated") => "0",
                    ("sysctl", "kern.hv_support") => "1",
                    ("sysctl", "hw.model") => "Mac15,6",
                    ("system_profiler", "-json") => {
                        r#"{"SPDisplaysDataType":[{"sppci_model":"Apple M3 Pro","spdisplays_mtlgpufamilysupport":"spdisplays_metal3","sppci_bus":"spdisplays_builtin"}]}"#
                    }
                    _ => return None,
                };
                Some(answer.as_bytes().to_vec())
            }),
        );
        let profile = probe_with(&context, Os::MacOs);
        assert_eq!(profile.os_name, "macOS 26.1");
        assert!(!profile.translated);
        assert_eq!(profile.virtualization.cpu_support, Some(true));
        assert_eq!(profile.gpus[0].name, "Apple M3 Pro");
        assert_eq!(profile.gpus[0].driver.as_deref(), Some("metal3"));
        assert_eq!(profile.gpus[0].vendor, crate::GpuVendor::Apple);
    }
}
