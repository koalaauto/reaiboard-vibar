//! 开机自启:用户级 LaunchAgent(~/Library/LaunchAgents),不需要管理员权限。
//! 只在用户在菜单里勾选时创建,取消勾选时删除;不会在后台悄悄安装任何东西。

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

/// 当前所在的 .app 包路径(从 Contents/MacOS/<exe> 往上三级);不在 .app 里则为 None
fn bundle_path(exe: &Path) -> Option<PathBuf> {
    let app = exe.parent()?.parent()?.parent()?;
    (app.extension()? == "app").then(|| app.to_path_buf())
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// 生成 LaunchAgent 内容。在 .app 里时用 `open -a` 启动,这样蓝牙/辅助功能权限按 App 身份计算。
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
