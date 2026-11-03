//! Launch at login through a user LaunchAgent (~/Library/LaunchAgents), no administrator rights.
//! It is only created when the user switches it on and removed when they switch it off.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

const LABEL: &str = "dev.koala.vibar";

fn plist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

pub fn enabled() -> bool {
    plist_path().exists()
}

/// Launch at login records the app's path, so it only makes sense once the app has been moved to
/// an Applications folder. Run from a disk image or the Downloads folder, the path would be
/// temporary and login would silently start nothing.
fn location_allows_autostart(exe: &Path, home: &str) -> bool {
    match bundle_path(exe) {
        // A bare binary is a development build, the user knows what they are doing.
        None => true,
        Some(app) => {
            app.starts_with("/Applications")
                || app.starts_with(Path::new(home).join("Applications"))
        }
    }
}

/// Whether the running app sits somewhere that launch at login can point to.
pub fn can_enable() -> bool {
    let home = std::env::var("HOME").unwrap_or_default();
    std::env::current_exe().is_ok_and(|exe| location_allows_autostart(&exe, &home))
}

/// The .app bundle the executable lives in (three levels up from Contents/MacOS/<exe>), or None
/// when it is not inside an .app.
fn bundle_path(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The LaunchAgent contents. Inside an .app it starts the bundle with `open -a`, so Bluetooth and
/// Accessibility permissions are attributed to the app.
fn render_plist(exe: &Path) -> String {
    let args: Vec<String> = match bundle_path(exe) {
        Some(app) => vec![
            "/usr/bin/open".into(),
            "-a".into(),
            app.display().to_string(),
        ],
        None => vec![exe.display().to_string()],
    };
    let items: String = args
        .iter()
        .map(|a| format!("    <string>{}</string>\n", xml_escape(a)))
        .collect();
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
<plist version=\"1.0\"><dict>\n\
  <key>Label</key><string>{LABEL}</string>\n\
  <key>ProgramArguments</key><array>\n{items}  </array>\n\
  <key>RunAtLoad</key><true/>\n\
</dict></plist>\n"
    )
}

pub fn set_enabled(on: bool) -> Result<()> {
    let path = plist_path();
    if on {
        let exe = std::env::current_exe().context("could not find the current executable path")?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, render_plist(&exe))
            .with_context(|| format!("could not write {}", path.display()))?;
    } else if path.exists() {
        std::fs::remove_file(&path)
            .with_context(|| format!("could not remove {}", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn autostart_is_only_allowed_from_an_applications_folder() {
        let h = "/Users/me";
        for ok in [
            "/Applications/Vibar.app/Contents/MacOS/vibar",
            "/Users/me/Applications/Vibar.app/Contents/MacOS/vibar",
            "/usr/local/bin/vibar", // a bare development binary
        ] {
            assert!(location_allows_autostart(Path::new(ok), h), "{ok}");
        }
        for bad in [
            "/Volumes/Vibar 0.1.0/Vibar.app/Contents/MacOS/vibar",
            "/Users/me/Downloads/Vibar.app/Contents/MacOS/vibar",
            "/private/var/folders/xx/AppTranslocation/ABC/d/Vibar.app/Contents/MacOS/vibar",
        ] {
            assert!(!location_allows_autostart(Path::new(bad), h), "{bad}");
        }
    }

    #[test]
    fn bundle_path_is_found_inside_app() {
        let exe = Path::new("/Applications/Vibar.app/Contents/MacOS/vibar");
        assert_eq!(
            bundle_path(exe).unwrap(),
            Path::new("/Applications/Vibar.app")
        );
        assert!(bundle_path(Path::new("/usr/local/bin/vibar")).is_none());
    }

    #[test]
    fn plist_launches_bundle_via_open_and_escapes_xml() {
        let p = render_plist(Path::new("/Apps/R&D Bar.app/Contents/MacOS/vibar"));
        assert!(p.contains("<string>/usr/bin/open</string>"));
        assert!(p.contains("<string>/Apps/R&amp;D Bar.app</string>"));
        assert!(p.contains("<key>RunAtLoad</key><true/>"));
    }

    #[test]
    fn plist_for_bare_binary_runs_it_directly() {
        let p = render_plist(Path::new("/usr/local/bin/vibar"));
        assert!(p.contains("<string>/usr/local/bin/vibar</string>"));
        assert!(!p.contains("/usr/bin/open"));
    }
}
