//! macOS application bundles: `Name.app/Contents/{Info.plist,MacOS/<exe>}`.

use std::path::Path;

use crate::{BundleDetails, Format, InspectError, Options, Report};

/// Inspect `path` as an application bundle; `None` if it is not one.
pub(crate) fn inspect_bundle(
    path: &Path,
    options: Options,
) -> Result<Option<Report>, InspectError> {
    let contents = path.join("Contents");
    let macos = contents.join("MacOS");
    if !macos.is_dir() {
        return Ok(None);
    }
    let plist = std::fs::read(contents.join("Info.plist")).unwrap_or_default();
    let identifier = plist_string(&plist, "CFBundleIdentifier");
    let executable = match plist_string(&plist, "CFBundleExecutable") {
        Some(name) => name,
        None => match single_file(&macos)? {
            Some(name) => name,
            None => return Ok(None),
        },
    };
    let inner_path = macos.join(&executable);
    let mut report = crate::inspect_path_with(&inner_path, options)?;
    if plist.starts_with(b"bplist00") && identifier.is_none() {
        report.notes.push(
            "Info.plist is a binary property list; MICA read the executable name from \
             Contents/MacOS"
                .into(),
        );
    }
    report.path = path.display().to_string();
    report.format = Format::AppBundle(BundleDetails {
        identifier,
        executable,
        inner: Box::new(report.format.clone()),
    });
    Ok(Some(report))
}

fn single_file(dir: &Path) -> Result<Option<String>, InspectError> {
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.push(entry.file_name().to_string_lossy().into_owned());
        }
        if files.len() > 1 {
            return Ok(None);
        }
    }
    Ok(files.pop())
}

/// The `<string>` value following `<key>name</key>` in an XML property list.
/// Binary property lists are not parsed (`None`).
pub(crate) fn plist_string(plist: &[u8], key: &str) -> Option<String> {
    let text = std::str::from_utf8(plist).ok()?;
    let marker = format!("<key>{key}</key>");
    let after = &text[text.find(&marker)? + marker.len()..];
    let after = after.trim_start();
    let value = after.strip_prefix("<string>")?;
    let end = value.find("</string>")?;
    Some(unescape(value[..end].trim()))
}

fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Arch, Platform};

    #[test]
    fn reads_xml_plists() {
        let plist = br#"<?xml version="1.0"?><plist><dict>
            <key>CFBundleExecutable</key>
            <string>My Game</string>
            <key>CFBundleIdentifier</key><string>com.example.game&amp;co</string>
        </dict></plist>"#;
        assert_eq!(
            plist_string(plist, "CFBundleExecutable").as_deref(),
            Some("My Game")
        );
        assert_eq!(
            plist_string(plist, "CFBundleIdentifier").as_deref(),
            Some("com.example.game&co")
        );
        assert_eq!(plist_string(plist, "Missing"), None);
        assert_eq!(
            plist_string(b"bplist00\x01\x02", "CFBundleExecutable"),
            None
        );
    }

    #[test]
    fn inspects_a_bundle_directory() {
        let root = std::env::temp_dir().join(format!("mica-bundle-{}", std::process::id()));
        let app = root.join("Demo.app");
        let macos = app.join("Contents").join("MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(
            app.join("Contents").join("Info.plist"),
            "<plist><dict><key>CFBundleExecutable</key><string>Demo</string>\
             <key>CFBundleIdentifier</key><string>org.example.demo</string></dict></plist>",
        )
        .unwrap();
        std::fs::write(
            macos.join("Demo"),
            crate::macho::tests::build(0x0100_000C, &[]),
        )
        .unwrap();
        let report = crate::inspect_path(&app).unwrap();
        assert_eq!(report.platform, Platform::MacOs);
        assert_eq!(report.architectures, vec![Arch::Aarch64]);
        match &report.format {
            Format::AppBundle(details) => {
                assert_eq!(details.executable, "Demo");
                assert_eq!(details.identifier.as_deref(), Some("org.example.demo"));
            }
            other => panic!("{other:?}"),
        }
        let plain = root.join("plain");
        std::fs::create_dir_all(&plain).unwrap();
        assert!(crate::inspect_path(&plain).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
