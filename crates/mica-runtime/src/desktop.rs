//! Optional Linux desktop integration: an "Open with MICA" entry for
//! Windows programs and installers in file managers.
//!
//! `mica desktop install` writes one desktop entry into the user's
//! applications directory; it appears under "Open With" and changes no
//! default. `--default` additionally makes it the default handler, after
//! recording the previous defaults so `mica desktop remove` restores them.

use std::path::{Path, PathBuf};

pub const ENTRY_NAME: &str = "mica-run.desktop";
const RECORD: &str = "desktop-defaults.txt";

/// MIME types of Windows executables, installers, and shortcuts as
/// shared-mime-info names them.
pub const MIME_TYPES: &[&str] = &[
    "application/x-ms-dos-executable",
    "application/x-msdownload",
    "application/vnd.microsoft.portable-executable",
    "application/x-msi",
    "application/x-ms-shortcut",
];

pub fn entry_contents(mica: &Path) -> String {
    let exec = mica.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        "[Desktop Entry]\n\
         # Created by MICA (mica desktop install); `mica desktop remove` deletes it.\n\
         Type=Application\n\
         Name=MICA\n\
         GenericName=Run Windows program\n\
         Comment=Open Windows programs and installers through MICA\n\
         Exec=\"{exec}\" open %f\n\
         Icon=application-x-executable\n\
         Terminal=false\n\
         NoDisplay=true\n\
         Categories=System;\n\
         MimeType={};\n",
        MIME_TYPES.join(";")
    )
}

pub fn applications_dir(home: &Path, data_home: Option<PathBuf>) -> PathBuf {
    data_home
        .unwrap_or_else(|| home.join(".local/share"))
        .join("applications")
}

/// Write the entry. Returns its path.
pub fn install(applications: &Path, mica: &Path) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(applications)?;
    let path = applications.join(ENTRY_NAME);
    std::fs::write(&path, entry_contents(mica))?;
    Ok(path)
}

/// Record the current default handlers (one `type=handler` per line) before
/// MICA replaces them.
pub fn record_defaults(state: &Path, defaults: &[(String, String)]) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(state)?;
    let path = state.join(RECORD);
    let text: String = defaults
        .iter()
        .map(|(mime, handler)| format!("{mime}={handler}\n"))
        .collect();
    std::fs::write(&path, text)?;
    Ok(path)
}

pub fn recorded_defaults(state: &Path) -> Vec<(String, String)> {
    std::fs::read_to_string(state.join(RECORD))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(mime, handler)| (mime.to_string(), handler.to_string()))
        .collect()
}

/// Delete the entry and the record. Returns whether the entry existed.
pub fn remove(applications: &Path, state: &Path) -> std::io::Result<bool> {
    let path = applications.join(ENTRY_NAME);
    let existed = path.is_file();
    if existed {
        let text = std::fs::read_to_string(&path).unwrap_or_default();
        if !text.contains("Created by MICA") {
            return Err(std::io::Error::other(format!(
                "{} was not created by MICA; leaving it alone",
                path.display()
            )));
        }
        std::fs::remove_file(&path)?;
    }
    let _ = std::fs::remove_file(state.join(RECORD));
    Ok(existed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_records_and_removes() {
        let root = std::env::temp_dir().join(format!("mica-desktop-{}", std::process::id()));
        let applications = applications_dir(&root, None);
        let state = root.join("state");
        let path = install(&applications, Path::new("/home/u/.local/bin/mica")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("Exec=\"/home/u/.local/bin/mica\" open %f"));
        assert!(text.contains("MimeType=application/x-ms-dos-executable;"));
        record_defaults(&state, &[("application/x-msi".into(), "wine.desktop".into())]).unwrap();
        assert_eq!(
            recorded_defaults(&state),
            vec![("application/x-msi".to_string(), "wine.desktop".to_string())]
        );
        assert!(remove(&applications, &state).unwrap());
        assert!(!path.exists());
        assert!(recorded_defaults(&state).is_empty());
        std::fs::write(&path, "[Desktop Entry]\nName=Someone else\n").unwrap();
        assert!(remove(&applications, &state).is_err());
        std::fs::remove_dir_all(&root).unwrap();
    }
}
