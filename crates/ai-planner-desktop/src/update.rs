//! Native-only update controls. The axum board (and any page that can reach it)
//! must never gain an endpoint that downloads and executes an installer.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use anyhow::{Context, Result};
use tauri::{
    menu::{Menu, MenuItem},
    App, AppHandle, Manager,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};

use ai_planner_update as release;

pub fn menu(app: &App) -> Result<()> {
    let handle = app.handle();
    let menu = Menu::default(handle)?;
    let check = MenuItem::with_id(
        handle,
        "check-updates",
        "Check for Updates…",
        true,
        None::<&str>,
    )?;
    let items = menu.items()?;
    let submenu = if cfg!(target_os = "macos") {
        items.first().and_then(|item| item.as_submenu())
    } else {
        items
            .iter()
            .find(|item| item.id().as_ref() == tauri::menu::HELP_SUBMENU_ID)
            .and_then(|item| item.as_submenu())
    }
    .context("the native application menu is missing")?;
    submenu.insert(&check, 1.min(submenu.items()?.len()))?;
    app.set_menu(menu)?;

    let busy = Arc::new(AtomicBool::new(false));
    app.on_menu_event(move |app, event| {
        if event.id() != check.id() || busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let app = app.clone();
        let item = check.clone();
        let busy = busy.clone();
        // Network, installer and blocking dialogs must not run on the event loop.
        std::thread::spawn(move || {
            if let Err(err) = check_and_update(&app, &item) {
                app.dialog()
                    .message(format!(
                        "{err:#}\n\nYou can retry Check for Updates or download a release from {}.",
                        release::RELEASES_URL
                    ))
                    .title("Update failed")
                    .kind(MessageDialogKind::Error)
                    .blocking_show();
            }
            if let Err(err) = item
                .set_text("Check for Updates…")
                .and_then(|()| item.set_enabled(true))
            {
                eprintln!("restoring the update menu: {err}");
            }
            if let Some(window) = app.get_webview_window("main") {
                if let Err(err) = window.set_title("ai-planner") {
                    eprintln!("restoring the window title: {err}");
                }
            }
            busy.store(false, Ordering::Release);
        });
    });
    Ok(())
}

fn check_and_update(app: &AppHandle, item: &MenuItem<tauri::Wry>) -> Result<()> {
    item.set_enabled(false)?;
    item.set_text("Checking for Updates…")?;
    let latest = release::latest_release_version()?;
    let running = env!("CARGO_PKG_VERSION");
    let bundle = if cfg!(target_os = "macos") {
        release::app_bundle(&std::env::current_exe()?).ok()
    } else {
        None
    };
    let installed = bundle.as_deref().map(release::app_version).transpose()?;
    match next_action(&latest, running, installed.as_deref())? {
        Action::Current => {
            app.dialog().message(format!("ai-planner {running} is up to date.\nLatest release: {latest}"))
                .title("Check for Updates").blocking_show();
        }
        Action::Restart => offer_restart(app, installed.as_deref().expect("installed app")),
        Action::Install => {
            if !confirm(app, &format!("Update ai-planner {running} to {latest}?\n\nThe download will be verified before replacing this app. Your database will be backed up first. A source-built CLI is left unchanged; use aip update to update the CLI too."), "Install Update", "Not Now") {
                return Ok(());
            }
            item.set_text("Installing Update…")?;
            if let Some(window) = app.get_webview_window("main") {
                window.set_title("ai-planner — Installing update…")?;
            }
            backup_database()?;
            release::install_release(&latest, None, bundle.as_deref())?;
            offer_restart(app, &latest);
        }
        Action::Download => {
            if confirm(app, &format!("ai-planner {latest} is available (running {running}).\n\nOpen the release page to download your platform's installer?"), "Open Downloads", "Not Now") {
                open_releases()?;
            }
        }
    }
    Ok(())
}

fn confirm(app: &AppHandle, message: &str, yes: &str, no: &str) -> bool {
    app.dialog()
        .message(message)
        .title("ai-planner Update")
        .buttons(MessageDialogButtons::OkCancelCustom(yes.into(), no.into()))
        .blocking_show()
}

fn offer_restart(app: &AppHandle, version: &str) {
    if confirm(
        app,
        &format!("ai-planner {version} is installed. Restart now to use it?"),
        "Restart Now",
        "Later",
    ) {
        app.restart();
    }
}

fn backup_database() -> Result<()> {
    let path = ai_planner_core::default_db_path();
    if path.exists() {
        let store = ai_planner_core::Store::open(&path)?;
        let stamp = ai_planner_core::util::now()
            .replace([':', '-'], "")
            .replace('Z', "");
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("database filename is not UTF-8")?;
        store.backup(&path.with_file_name(format!("{name}.{stamp}.pre-update.bak")))?;
    }
    Ok(())
}

fn open_releases() -> Result<()> {
    #[cfg(target_os = "macos")]
    let mut command = std::process::Command::new("open");
    #[cfg(target_os = "linux")]
    let mut command = std::process::Command::new("xdg-open");
    #[cfg(target_os = "windows")]
    let mut command = {
        let mut cmd = std::process::Command::new("rundll32");
        cmd.arg("url.dll,FileProtocolHandler");
        cmd
    };
    let status = command
        .arg(release::RELEASES_URL)
        .status()
        .context("opening the release page")?;
    anyhow::ensure!(status.success(), "could not open {}", release::RELEASES_URL);
    Ok(())
}

#[derive(Debug, PartialEq)]
enum Action {
    Current,
    Restart,
    Install,
    Download,
}

fn next_action(latest: &str, running: &str, installed: Option<&str>) -> Result<Action> {
    if let Some(installed) = installed {
        if release::needs_update(installed, running, false)?
            && !release::needs_update(latest, installed, false)?
        {
            return Ok(Action::Restart);
        }
    }
    if !release::needs_update(latest, running, false)? {
        return Ok(Action::Current);
    }
    Ok(if installed.is_some() {
        Action::Install
    } else {
        Action::Download
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_installed_mac_app_can_update_but_an_unbundled_or_other_platform_app_downloads() {
        assert_eq!(
            next_action("0.6.0", "0.5.1", Some("0.5.1")).unwrap(),
            Action::Install
        );
        assert_eq!(
            next_action("0.6.0", "0.5.1", None).unwrap(),
            Action::Download
        );
    }

    #[test]
    fn a_cli_update_while_the_window_is_open_only_needs_a_restart() {
        assert_eq!(
            next_action("0.6.0", "0.5.1", Some("0.6.0")).unwrap(),
            Action::Restart
        );
        assert_eq!(
            next_action("0.6.0", "0.5.1", Some("0.7.0")).unwrap(),
            Action::Restart
        );
    }

    #[test]
    fn a_current_or_ahead_of_release_app_is_never_downgraded() {
        assert_eq!(
            next_action("0.5.1", "0.5.1", Some("0.5.1")).unwrap(),
            Action::Current
        );
        assert_eq!(
            next_action("0.5.1", "0.6.0", Some("0.6.0")).unwrap(),
            Action::Current
        );
        assert!(next_action("invalid", "0.5.1", None).is_err());
    }
}
