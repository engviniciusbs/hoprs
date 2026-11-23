//! Every user-visible string, in one place (i18n-ready). Sentence case.

use crate::i18n::{tr, tr_args};

pub fn app_title() -> &'static str {
    tr("app-title")
}
pub fn already_running() -> &'static str {
    tr("already-running")
}
pub fn cannot_open_profiles() -> &'static str {
    tr("cannot-open-profiles")
}

// Top bar.
pub fn brand() -> &'static str {
    tr("brand")
}
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn no_sessions() -> &'static str {
    tr("no-sessions")
}
pub fn go_to_anything() -> &'static str {
    tr("go-to-anything")
}
pub fn go_to_anything_key() -> &'static str {
    tr("go-to-anything-key")
}
/// The tooltip of the sidebar's ⋯ button.
pub fn sidebar_options() -> &'static str {
    tr("sidebar-options")
}

// Context menus.
pub fn menu_show_workspaces() -> &'static str {
    tr("menu-show-workspaces")
}
pub fn menu_open() -> &'static str {
    tr("menu-open")
}
pub fn menu_copy_path() -> &'static str {
    tr("menu-copy-path")
}
pub fn menu_refresh() -> &'static str {
    tr("menu-refresh")
}
pub fn menu_show_in_panes() -> &'static str {
    tr("menu-show-in-panes")
}
pub fn menu_open_files() -> &'static str {
    tr("menu-open-files")
}
pub fn menu_close_other_tabs() -> &'static str {
    tr("menu-close-other-tabs")
}
pub fn key_refresh() -> &'static str {
    tr("key-refresh")
}
pub fn menu_show_sites() -> &'static str {
    tr("menu-show-sites")
}

// The Ctrl+K palette.
pub fn palette_placeholder() -> &'static str {
    tr("palette-placeholder")
}
pub fn palette_workspaces_placeholder() -> &'static str {
    tr("palette-workspaces-placeholder")
}
pub fn palette_no_results() -> &'static str {
    tr("palette-no-results")
}
pub fn palette_group_action() -> &'static str {
    tr("palette-group-action")
}
pub fn palette_group_tab() -> &'static str {
    tr("palette-group-tab")
}
pub fn palette_group_site() -> &'static str {
    tr("palette-group-site")
}
pub fn palette_group_workspace() -> &'static str {
    tr("palette-group-workspace")
}
pub fn palette_kind_terminal() -> &'static str {
    tr("palette-kind-terminal")
}
pub fn palette_kind_files() -> &'static str {
    tr("palette-kind-files")
}
pub fn palette_toggle_queue() -> &'static str {
    tr("palette-toggle-queue")
}
pub fn palette_toggle_log() -> &'static str {
    tr("palette-toggle-log")
}
pub fn palette_close_tab() -> &'static str {
    tr("palette-close-tab")
}
pub fn palette_close_all(name: &str) -> String {
    tr_args("palette-close-all", &[("name", name.into())])
}
pub fn update_palette(version: &str) -> String {
    tr_args("update-palette", &[("version", version.into())])
}

// The update card, its footer line and Settings → General (Plan 15).
pub fn update_available(version: &str) -> String {
    tr_args("update-available", &[("version", version.into())])
}
pub fn update_you_have(version: &str) -> String {
    tr_args("update-you-have", &[("version", version.into())])
}
pub fn update_action() -> &'static str {
    tr("update-action")
}
pub fn update_whats_new() -> &'static str {
    tr("update-whats-new")
}
pub fn update_dismiss() -> &'static str {
    tr("update-dismiss")
}
pub fn update_downloading(done: &str, total: &str) -> String {
    tr_args(
        "update-downloading",
        &[("done", done.into()), ("total", total.into())],
    )
}
/// The download's progress while the server gave no size.
pub fn update_downloading_size(done: &str) -> String {
    tr_args("update-downloading-size", &[("done", done.into())])
}
pub fn update_verifying() -> &'static str {
    tr("update-verifying")
}
pub fn update_try_again() -> &'static str {
    tr("update-try-again")
}
pub fn update_ready_windows() -> &'static str {
    tr("update-ready-windows")
}
pub fn update_transfers_stop(count: usize) -> String {
    tr_args("update-transfers-stop", &[("count", count.into())])
}
pub fn update_close_and_install() -> &'static str {
    tr("update-close-and-install")
}
pub fn update_later() -> &'static str {
    tr("update-later")
}
/// Windows, after "Close and install": the installer opens and hoprs closes.
pub fn update_installing() -> &'static str {
    tr("update-installing")
}
pub fn update_linux_opened() -> &'static str {
    tr("update-linux-opened")
}
/// Linux, the system installer failed to open: the command is the way.
pub fn update_linux_run() -> &'static str {
    tr("update-linux-run")
}
/// Linux and macOS: the installer did not open (the error itself in the tooltip).
pub fn update_open_failed() -> &'static str {
    tr("update-open-failed")
}
/// Windows, the installer open (a close the user cancelled).
pub fn update_installer_open() -> &'static str {
    tr("update-installer-open")
}
pub fn update_close_hoprs() -> &'static str {
    tr("update-close-hoprs")
}
pub fn update_copy() -> &'static str {
    tr("update-copy")
}
pub fn update_macos_steps() -> &'static str {
    tr("update-macos-steps")
}
pub fn update_footer_available(version: &str) -> String {
    tr_args("update-footer-available", &[("version", version.into())])
}
pub fn update_footer_install(version: &str) -> String {
    tr_args("update-footer-install", &[("version", version.into())])
}
pub fn settings_update_check() -> &'static str {
    tr("settings-update-check")
}
pub fn settings_update_check_hint() -> &'static str {
    tr("settings-update-check-hint")
}
pub fn settings_check_now() -> &'static str {
    tr("settings-check-now")
}
pub fn settings_update_checking() -> &'static str {
    tr("settings-update-checking")
}
pub fn settings_up_to_date() -> &'static str {
    tr("settings-up-to-date")
}
pub fn workspace_open_tip() -> &'static str {
    tr("workspace-open-tip")
}
pub fn palette_open_workspace() -> &'static str {
    tr("palette-open-workspace")
}
pub fn palette_edit() -> &'static str {
    tr("palette-edit")
}
pub fn palette_compare_folders() -> &'static str {
    tr("palette-compare-folders")
}
pub fn palette_upload() -> &'static str {
    tr("palette-upload")
}
pub fn palette_download() -> &'static str {
    tr("palette-download")
}
pub fn palette_copy() -> &'static str {
    tr("palette-copy")
}
pub fn palette_paste() -> &'static str {
    tr("palette-paste")
}
pub fn palette_find() -> &'static str {
    tr("palette-find")
}
pub fn palette_key_copy() -> &'static str {
    tr("palette-key-copy")
}
pub fn palette_key_paste() -> &'static str {
    tr("palette-key-paste")
}
pub fn palette_key_find() -> &'static str {
    tr("palette-key-find")
}

// Sidebar.
pub fn filter_sites() -> &'static str {
    tr("filter-sites")
}
pub fn workspaces() -> &'static str {
    tr("workspaces")
}
pub fn all_sites() -> &'static str {
    tr("all-sites")
}
/// The sidebar's note when both sections are hidden.
pub fn sidebar_all_hidden() -> &'static str {
    tr("sidebar-all-hidden")
}
pub fn sidebar_collapse() -> &'static str {
    tr("sidebar-collapse")
}
pub fn sidebar_expand() -> &'static str {
    tr("sidebar-expand")
}
pub fn no_workspaces() -> &'static str {
    tr("no-workspaces")
}
pub fn no_sites() -> &'static str {
    tr("no-sites")
}
pub fn import_filezilla() -> &'static str {
    tr("import-filezilla")
}
pub fn settings() -> &'static str {
    tr("settings")
}
/// The key that opens Settings, as the palette shows it (Cmd on macOS).
pub fn key_settings() -> &'static str {
    if cfg!(target_os = "macos") {
        tr("key-settings-macos")
    } else {
        tr("key-settings")
    }
}
/// The palette's row that closes the settings tab.
pub fn palette_close_settings() -> &'static str {
    tr("palette-close-settings")
}
pub fn new_site() -> &'static str {
    tr("new-site")
}
pub fn no_matches() -> &'static str {
    tr("no-matches")
}
pub fn menu_open_all() -> &'static str {
    tr("menu-open-all")
}
pub fn edit() -> &'static str {
    tr("edit")
}
pub fn site_missing() -> &'static str {
    tr("site-missing")
}
pub fn cannot_load_sites() -> &'static str {
    tr("cannot-load-sites")
}
pub fn retry() -> &'static str {
    tr("retry")
}
pub fn unreadable_entry() -> &'static str {
    tr("unreadable-entry")
}
pub fn open_profile_folder() -> &'static str {
    tr("open-profile-folder")
}
/// Opening the profile folder failed.
pub fn cannot_open_folder(reason: &str) -> String {
    tr_args("cannot-open-folder", &[("reason", reason.into())])
}

/// "1 site", "3 sites".
pub fn site_count(n: usize) -> String {
    tr_args("site-count", &[("count", n.into())])
}

/// A workspace's count: "3 sites", or "2 of 3 sites" when some are missing.
pub fn workspace_site_count(present: usize, total: usize) -> String {
    if present == total {
        site_count(total)
    } else {
        tr_args(
            "workspace-site-count-partial",
            &[("present", present.into()), ("total", total.into())],
        )
    }
}

/// An unreadable entry: its name, or where it is.
pub fn unreadable_name(name: Option<&str>, file: &str, index: usize) -> String {
    match name {
        Some(name) => name.to_owned(),
        None => tr_args(
            "unreadable-name-fallback",
            &[
                ("number", (index + 1).to_string().as_str().into()),
                ("file", file.into()),
            ],
        ),
    }
}

// Site form.
pub fn site_form_new() -> &'static str {
    tr("site-form-new")
}
pub fn site_form_edit() -> &'static str {
    tr("site-form-edit")
}
pub fn field_name() -> &'static str {
    tr("field-name")
}
pub fn field_name_hint() -> &'static str {
    tr("field-name-hint")
}
pub fn field_host() -> &'static str {
    tr("field-host")
}
pub fn field_port() -> &'static str {
    tr("field-port")
}
pub fn field_user() -> &'static str {
    tr("field-user")
}
pub fn field_protocol() -> &'static str {
    tr("field-protocol")
}
pub fn field_password() -> &'static str {
    tr("field-password")
}
pub fn password_optional() -> &'static str {
    tr("password-optional")
}
pub fn password_replace() -> &'static str {
    tr("password-replace")
}
pub fn password_saved() -> &'static str {
    tr("password-saved")
}
pub fn forget_password() -> &'static str {
    tr("forget-password")
}
pub fn ssh_options() -> &'static str {
    tr("ssh-options")
}
pub fn use_agent() -> &'static str {
    tr("use-agent")
}
pub fn key_file() -> &'static str {
    tr("key-file")
}
pub fn key_file_hint() -> &'static str {
    tr("key-file-hint")
}
pub fn open_as() -> &'static str {
    tr("open-as")
}
pub fn open_as_terminal() -> &'static str {
    tr("open-as-terminal")
}
pub fn open_as_files() -> &'static str {
    tr("open-as-files")
}
pub fn open_as_both() -> &'static str {
    tr("open-as-both")
}
pub fn ftp_options() -> &'static str {
    tr("ftp-options")
}
pub fn ftp_mode() -> &'static str {
    tr("ftp-mode")
}
pub fn ftp_passive() -> &'static str {
    tr("ftp-passive")
}
pub fn ftp_active() -> &'static str {
    tr("ftp-active")
}
pub fn nat_workaround() -> &'static str {
    tr("nat-workaround")
}
pub fn start_folders() -> &'static str {
    tr("start-folders")
}
pub fn remote_dir() -> &'static str {
    tr("remote-dir")
}
pub fn agents_section() -> &'static str {
    tr("agents-section")
}
pub fn agent_access_none() -> &'static str {
    tr("agent-access-none")
}
pub fn agent_access_read() -> &'static str {
    tr("agent-access-read")
}
pub fn agent_access_all() -> &'static str {
    tr("agent-access-all")
}
pub fn agent_access_read_hint() -> &'static str {
    tr("agent-access-read-hint")
}
pub fn agent_access_all_hint_ssh() -> &'static str {
    tr("agent-access-all-hint-ssh")
}
pub fn agent_access_all_hint_ftp() -> &'static str {
    tr("agent-access-all-hint-ftp")
}
pub fn settings_agents_description() -> &'static str {
    tr("settings-agents-description")
}
pub fn agents_allow() -> &'static str {
    tr("agents-allow")
}
pub fn agents_allow_hint() -> &'static str {
    tr("agents-allow-hint")
}
pub fn agents_claude_code() -> &'static str {
    tr("agents-claude-code")
}
pub fn agents_other_clients() -> &'static str {
    tr("agents-other-clients")
}
pub fn agents_levels(read: usize, all: usize) -> String {
    tr_args(
        "agents-levels",
        &[("read", read.into()), ("all", all.into())],
    )
}
/// The status bar's indicator while an agent is connected.
pub fn agent_connected() -> &'static str {
    tr("agent-connected")
}
pub fn agent_connected_tip(count: usize) -> String {
    tr_args("agent-connected-tip", &[("count", count.into())])
}
/// The tag of an agent's transfer in the queue.
pub fn queue_agent_tag() -> &'static str {
    tr("queue-agent-tag")
}
/// A line of a security dialog whose connection an agent asked for.
pub fn prompt_by_agent() -> &'static str {
    tr("prompt-by-agent")
}
/// The tooltip of a site only an agent's session holds.
pub fn site_connected_by_agent() -> &'static str {
    tr("site-connected-by-agent")
}
pub fn local_dir() -> &'static str {
    tr("local-dir")
}
pub fn folder_hint() -> &'static str {
    tr("folder-hint")
}
pub fn host_required() -> &'static str {
    tr("host-required")
}
pub fn port_invalid() -> &'static str {
    tr("port-invalid")
}
pub fn save() -> &'static str {
    tr("save")
}
pub fn saving() -> &'static str {
    tr("saving")
}
pub fn cancel() -> &'static str {
    tr("cancel")
}
pub fn delete() -> &'static str {
    tr("delete")
}
pub fn keep() -> &'static str {
    tr("keep")
}
pub fn delete_site_confirm() -> &'static str {
    tr("delete-site-confirm")
}

// Workspace form.
pub fn workspace_form_new() -> &'static str {
    tr("workspace-form-new")
}
pub fn workspace_form_edit() -> &'static str {
    tr("workspace-form-edit")
}
pub fn workspace_sites() -> &'static str {
    tr("workspace-sites")
}
pub fn workspace_no_sites() -> &'static str {
    tr("workspace-no-sites")
}
pub fn name_required() -> &'static str {
    tr("name-required")
}
pub fn delete_workspace_confirm() -> &'static str {
    tr("delete-workspace-confirm")
}

// Center.
pub fn no_session_title() -> &'static str {
    tr("no-session-title")
}
pub fn no_session_hint() -> &'static str {
    tr("no-session-hint")
}
pub fn keyring_unavailable() -> &'static str {
    tr("keyring-unavailable")
}
pub fn edit_site() -> &'static str {
    tr("edit-site")
}

// Tabs.
pub fn close_tab() -> &'static str {
    tr("close-tab")
}
/// The tab strip's actions on a connected SSH tab: another tab on its session.
pub fn new_terminal_tab() -> &'static str {
    tr("new-terminal-tab")
}
pub fn new_files_tab() -> &'static str {
    tr("new-files-tab")
}
pub fn terminal_overflowed() -> &'static str {
    tr("terminal-overflowed")
}
pub fn terminal_reopen() -> &'static str {
    tr("terminal-reopen")
}
pub fn terminal_restart() -> &'static str {
    tr("terminal-restart")
}
pub fn terminal_shell_closed() -> &'static str {
    tr("terminal-shell-closed")
}
pub fn terminal_open_failed() -> &'static str {
    tr("terminal-open-failed")
}
pub fn terminal_retry() -> &'static str {
    tr("terminal-retry")
}
pub fn terminal_waiting_for_connection() -> &'static str {
    tr("terminal-waiting-for-connection")
}
pub fn terminal_input_dropped() -> &'static str {
    tr("terminal-input-dropped")
}
/// Written into the scrollback when a new shell starts after a reconnect.
pub fn terminal_reconnected() -> &'static str {
    tr("terminal-reconnected")
}
pub fn terminal_search_placeholder() -> &'static str {
    tr("terminal-search-placeholder")
}
pub fn terminal_search_next() -> &'static str {
    tr("terminal-search-next")
}
pub fn terminal_search_previous() -> &'static str {
    tr("terminal-search-previous")
}
pub fn terminal_search_close() -> &'static str {
    tr("terminal-search-close")
}
pub fn terminal_search_no_match() -> &'static str {
    tr("terminal-search-no-match")
}
/// "Shell exited (code N)", or without a code when the server sent none.
pub fn shell_exited(code: Option<u32>) -> String {
    match code {
        Some(code) => tr_args(
            "shell-exited-code",
            &[("code", code.to_string().as_str().into())],
        ),
        None => tr("shell-exited").to_owned(),
    }
}

// File panes.
pub fn local() -> &'static str {
    tr("local")
}
pub fn col_name() -> &'static str {
    tr("col-name")
}
pub fn col_size() -> &'static str {
    tr("col-size")
}
pub fn col_modified() -> &'static str {
    tr("col-modified")
}
pub fn col_permissions() -> &'static str {
    tr("col-permissions")
}
pub fn sort_ascending() -> &'static str {
    tr("sort-ascending")
}
pub fn sort_descending() -> &'static str {
    tr("sort-descending")
}
/// The row that goes up one folder.
pub fn parent_dir() -> &'static str {
    tr("parent-dir")
}
/// After a folder name, and between breadcrumb segments.
pub fn path_separator() -> &'static str {
    tr("path-separator")
}
/// The breadcrumb's first segment for the home folder.
pub fn home_dir() -> &'static str {
    tr("home-dir")
}
/// A drag from the local pane over the remote one: "Upload 2 items to /srv".
pub fn drop_upload(count: usize, folder: &str) -> String {
    tr_args(
        "drop-upload",
        &[("count", count.into()), ("folder", folder.into())],
    )
}
/// A drag from the remote pane over the local one: "Download 1 item to ~/Downloads".
pub fn drop_download(count: usize, folder: &str) -> String {
    tr_args(
        "drop-download",
        &[("count", count.into()), ("folder", folder.into())],
    )
}
/// Files from the system over the remote pane: "Upload 3 files to /srv".
pub fn drop_upload_files(count: usize, folder: &str) -> String {
    tr_args(
        "drop-upload-files",
        &[("count", count.into()), ("folder", folder.into())],
    )
}
/// Files from the system over the remote pane, their count not known yet.
pub fn drop_upload_files_unknown(folder: &str) -> String {
    tr_args("drop-upload-files-unknown", &[("folder", folder.into())])
}
/// The drag chip, after the first name: "+1" for one more item.
pub fn drag_more(more: usize) -> String {
    tr_args("drag-more", &[("more", more.into())])
}
/// The drag chip's badge: how many items go ("2").
pub fn drag_count(count: usize) -> String {
    tr_args("drag-count", &[("count", count.into())])
}
/// The size column of a folder.
pub fn no_size() -> &'static str {
    tr("no-size")
}
pub fn empty_folder() -> &'static str {
    tr("empty-folder")
}
pub fn reconnect() -> &'static str {
    tr("reconnect")
}
/// Units of [`crate::ui::format::size`], base 1024, in `lang`.
pub fn size_unit_in(lang: crate::i18n::Language, i: usize) -> &'static str {
    let cat = crate::i18n::catalog_for(lang);
    match i {
        0 => cat.tr("size-unit-b"),
        1 => cat.tr("size-unit-kb"),
        2 => cat.tr("size-unit-mb"),
        3 => cat.tr("size-unit-gb"),
        4 => cat.tr("size-unit-tb"),
        5 => cat.tr("size-unit-pb"),
        6 => cat.tr("size-unit-eb"),
        _ => "",
    }
}
/// Short month names for file dates, in `lang`.
pub fn month_in(lang: crate::i18n::Language, m0: u32) -> &'static str {
    let cat = crate::i18n::catalog_for(lang);
    match m0 {
        0 => cat.tr("month-1"),
        1 => cat.tr("month-2"),
        2 => cat.tr("month-3"),
        3 => cat.tr("month-4"),
        4 => cat.tr("month-5"),
        5 => cat.tr("month-6"),
        6 => cat.tr("month-7"),
        7 => cat.tr("month-8"),
        8 => cat.tr("month-9"),
        9 => cat.tr("month-10"),
        10 => cat.tr("month-11"),
        11 => cat.tr("month-12"),
        _ => "",
    }
}

// Queue.
pub fn transfers() -> &'static str {
    tr("transfers")
}
pub fn queue_active() -> &'static str {
    tr("queue-active")
}
pub fn queue_queued() -> &'static str {
    tr("queue-queued")
}
pub fn queue_failed() -> &'static str {
    tr("queue-failed")
}
pub fn queue_done() -> &'static str {
    tr("queue-done")
}
pub fn queue_hide() -> &'static str {
    tr("queue-hide")
}
pub fn queue_show() -> &'static str {
    tr("queue-show")
}
pub fn no_transfers() -> &'static str {
    tr("no-transfers")
}
pub fn no_transfers_hint() -> &'static str {
    tr("no-transfers-hint")
}
pub fn queue_loading() -> &'static str {
    tr("queue-loading")
}
pub fn nothing_active() -> &'static str {
    tr("nothing-active")
}
pub fn nothing_queued() -> &'static str {
    tr("nothing-queued")
}
pub fn nothing_failed() -> &'static str {
    tr("nothing-failed")
}
pub fn nothing_done() -> &'static str {
    tr("nothing-done")
}
/// Removes done, skipped, cancelled and failed rows (the engine's `clear_finished`).
pub fn clear_finished() -> &'static str {
    tr("clear-finished")
}
// Technical log panel.
pub fn log() -> &'static str {
    tr("log")
}
pub fn log_error() -> &'static str {
    tr("log-error")
}
pub fn log_warn() -> &'static str {
    tr("log-warn")
}
pub fn log_info() -> &'static str {
    tr("log-info")
}
pub fn log_debug() -> &'static str {
    tr("log-debug")
}
pub fn filter_log() -> &'static str {
    tr("filter-log")
}
/// Copies the lines the filters show (all of them without a filter).
pub fn copy_shown() -> &'static str {
    tr("copy-shown")
}
pub fn clear() -> &'static str {
    tr("clear")
}
pub fn clear_hint() -> &'static str {
    tr("clear-hint")
}
pub fn log_level_hint() -> &'static str {
    tr("log-level-hint")
}
pub fn no_log_lines() -> &'static str {
    tr("no-log-lines")
}
pub fn no_log_matches() -> &'static str {
    tr("no-log-matches")
}

/// The log panel's count: "1 line", "120 lines".
pub fn log_lines(n: usize) -> String {
    tr_args("log-lines", &[("count", n.into())])
}

pub fn waiting_for_worker() -> &'static str {
    tr("waiting-for-worker")
}
pub fn starting() -> &'static str {
    tr("starting")
}
/// A folder whose files are being copied.
pub fn transferring() -> &'static str {
    tr("transferring")
}
/// A failed row the user dismissed (still failed in the engine; Retry stays).
pub fn dismissed() -> &'static str {
    tr("dismissed")
}
pub fn paused() -> &'static str {
    tr("paused")
}
pub fn cancelled() -> &'static str {
    tr("cancelled")
}
pub fn transfer_done() -> &'static str {
    tr("transfer-done")
}
pub fn file_exists() -> &'static str {
    tr("file-exists")
}
pub fn overwrite() -> &'static str {
    tr("overwrite")
}
pub fn resume() -> &'static str {
    tr("resume")
}
pub fn skip() -> &'static str {
    tr("skip")
}
pub fn pause() -> &'static str {
    tr("pause")
}
/// The engine also makes the answer its policy for every later conflict of this run.
pub fn apply_to_all() -> &'static str {
    tr("apply-to-all")
}
/// After an edit's file name once a save went up: "saved 14:02" (a dash precedes it).
pub fn saved_at(time: &str) -> String {
    tr_args("saved-at", &[("time", time.into())])
}
pub fn site_paused() -> &'static str {
    tr("site-paused")
}

/// A queue tab: "Active 3".
pub fn queue_tab(label: &str, count: usize) -> String {
    format!("{label} {count}")
}

/// "41.7 MB/s".
pub fn speed(bytes_per_sec: u64) -> String {
    format!("{}/s", crate::ui::format::size(bytes_per_sec))
}

/// After the total speed: " total, 4 workers".
pub fn total_workers(workers: usize) -> String {
    // The leading space is part of the text (it follows the speed); Fluent trims it, so it is added here.
    format!(
        " {}",
        tr_args("total-workers", &[("workers", workers.into())])
    )
}

/// "51%".
pub fn percent(fraction: f32) -> String {
    format!("{:.0}%", (fraction * 100.0).floor())
}

/// A file of unknown size: "12 MB, 3.4 MB/s".
pub fn bytes_and_speed(done: u64, bytes_per_sec: u64) -> String {
    format!(
        "{}, {}",
        crate::ui::format::size(done),
        speed(bytes_per_sec)
    )
}

/// In the queue head once an answer went "to all": "Conflicts: overwrite (this session)".
pub fn session_policy(policy: hoprs_transfer::ConflictPolicy) -> String {
    use hoprs_transfer::ConflictPolicy::*;
    let name = match policy {
        Ask => tr("policy-ask"),
        Overwrite => tr("policy-overwrite"),
        Skip => tr("policy-skip"),
        OverwriteIfNewer => tr("policy-overwrite-if-newer"),
        Resume => tr("policy-resume"),
    };
    tr_args("session-policy", &[("name", name.into())])
}

/// The same in a narrow window, as a short mono tag: "overwrite: session".
pub fn session_policy_tag(policy: hoprs_transfer::ConflictPolicy) -> String {
    use hoprs_transfer::ConflictPolicy::*;
    let name = match policy {
        Ask => tr("policy-ask"),
        Overwrite => tr("policy-overwrite"),
        Skip => tr("policy-skip"),
        OverwriteIfNewer => tr("policy-if-newer"),
        Resume => tr("policy-resume"),
    };
    tr_args("session-policy-tag", &[("name", name.into())])
}

/// The queue database could not be opened: the engine runs on an in-memory queue.
pub fn queue_unavailable(reason: &str) -> String {
    tr_args("queue-unavailable", &[("reason", reason.into())])
}

/// The text of an error in the user's language (see `error_in`).
pub fn error(err: &hoprs_core::Error) -> String {
    error_in(crate::i18n::catalog_for(crate::i18n::language()), err)
}

/// The text of an error in `catalog`'s language. An error with a code is looked up as
/// `error-<code>` with its arguments; it falls back to the English `message` when there
/// is no code, no such key, or the translation needs an argument the error lacks. This is
/// the one sanctioned computed catalog key: the coverage tests ignore it and check
/// the codes themselves.
pub fn error_in(catalog: &crate::i18n::Catalog, err: &hoprs_core::Error) -> String {
    let Some(code) = err.code else {
        return err.message.clone();
    };
    let args: Vec<(&str, crate::i18n::Arg)> = err
        .args
        .iter()
        .map(|(name, value)| (*name, crate::i18n::Arg::Str(value)))
        .collect();
    catalog
        .tr_args_checked(&format!("error-{code}"), &args)
        .unwrap_or_else(|| err.message.clone())
}

/// A skip reason the engine stored, in the user's language; an unknown one passes through.
pub fn skip_reason(raw: &str) -> String {
    match raw {
        "exists" => tr("skip-exists").to_owned(),
        "not newer" => tr("skip-not-newer").to_owned(),
        "already complete" => tr("skip-already-complete").to_owned(),
        "skipped by user" => tr("skip-by-user").to_owned(),
        "symlinks are not followed" => tr("skip-symlink").to_owned(),
        other => other.to_owned(),
    }
}

/// "Skipped: exists".
pub fn skipped(reason: &str) -> String {
    tr_args(
        "skipped",
        &[("reason", skip_reason(reason).as_str().into())],
    )
}

// Status bar.
pub fn not_connected() -> &'static str {
    tr("not-connected")
}

// Status bar and connection states.
pub fn connecting() -> &'static str {
    tr("connecting")
}
pub fn connected() -> &'static str {
    tr("connected")
}
pub fn waiting_for_you() -> &'static str {
    tr("waiting-for-you")
}
pub fn closing() -> &'static str {
    tr("closing")
}
pub fn menu_close_all() -> &'static str {
    tr("menu-close-all")
}
pub fn reconnecting_now() -> &'static str {
    tr("reconnecting-now")
}

/// A workspace's saved folder is gone; the pane shows a folder above it instead.
pub fn saved_folder_gone(saved: &str) -> String {
    tr_args("saved-folder-gone", &[("saved", saved.into())])
}

/// "Reconnecting (2/5)".
pub fn reconnecting(attempt: u32, of: u32) -> String {
    tr_args(
        "reconnecting",
        &[
            ("attempt", attempt.to_string().as_str().into()),
            ("of", of.to_string().as_str().into()),
        ],
    )
}

/// "Disconnected — Connection lost".
pub fn disconnected(reason: &str) -> String {
    tr_args("disconnected", &[("reason", reason.into())])
}

/// `user@host:port`.
pub fn endpoint(user: &str, host: &str, port: u16) -> String {
    format!("{user}@{host}:{port}")
}

/// How an SSH session logged in: "ed25519 via ssh-agent", "rsa key", "password"...
pub fn ssh_auth(method: &hoprs_ssh::AuthMethod) -> String {
    use hoprs_ssh::AuthMethod;
    match method {
        AuthMethod::None => tr("auth-none").to_owned(),
        AuthMethod::Agent { key_type } => tr_args("auth-agent", &[("key_type", key_type.into())]),
        AuthMethod::KeyFile { key_type } => {
            tr_args("auth-key-file", &[("key_type", key_type.into())])
        }
        AuthMethod::Password => tr("auth-password").to_owned(),
        AuthMethod::KeyboardInteractive => tr("auth-keyboard-interactive").to_owned(),
    }
}

/// An FTP site's security, in place of a login method.
pub fn ftp_auth(protocol: hoprs_core::Protocol) -> Option<&'static str> {
    use hoprs_core::Protocol;
    match protocol {
        Protocol::Ssh => None,
        Protocol::Ftp => Some(tr("ftp-auth-ftp")),
        Protocol::FtpsExplicit => Some(tr("ftp-auth-ftps-explicit")),
        Protocol::FtpsImplicit => Some(tr("ftp-auth-ftps-implicit")),
    }
}

/// "3 of 3 sites in Shop".
pub fn workspace_status(connected: usize, total: usize, name: &str) -> String {
    tr_args(
        "workspace-status",
        &[
            ("connected", connected.into()),
            ("sites", site_count(total).as_str().into()),
            ("name", name.into()),
        ],
    )
}

// File actions.
pub fn new_folder() -> &'static str {
    tr("new-folder")
}
pub fn rename() -> &'static str {
    tr("rename")
}
pub fn permissions() -> &'static str {
    tr("permissions")
}
pub fn key_new_folder() -> &'static str {
    tr("key-new-folder")
}
pub fn key_rename() -> &'static str {
    tr("key-rename")
}
pub fn key_delete() -> &'static str {
    tr("key-delete")
}
pub fn key_edit() -> &'static str {
    tr("key-edit")
}
pub fn upload() -> &'static str {
    tr("upload")
}
pub fn download() -> &'static str {
    tr("download")
}
pub fn key_transfer() -> &'static str {
    tr("key-transfer")
}
/// The name a new folder starts with.
pub fn new_folder_name() -> &'static str {
    tr("new-folder-name")
}
pub fn name_empty() -> &'static str {
    tr("name-empty")
}
pub fn name_invalid() -> &'static str {
    tr("name-invalid")
}
pub fn name_invalid_windows() -> &'static str {
    tr("name-invalid-windows")
}
pub fn name_spaces() -> &'static str {
    tr("name-spaces")
}
pub fn mixed_permissions() -> &'static str {
    tr("mixed-permissions")
}
pub fn name_taken() -> &'static str {
    tr("name-taken")
}
pub fn dismiss() -> &'static str {
    tr("dismiss")
}
pub fn creating_folder() -> &'static str {
    tr("creating-folder")
}
pub fn renaming() -> &'static str {
    tr("renaming")
}
pub fn changing_permissions() -> &'static str {
    tr("changing-permissions")
}
pub fn delete_folders_note() -> &'static str {
    tr("delete-folders-note")
}
pub fn permissions_title() -> &'static str {
    tr("permissions-title")
}
pub fn perm_owner() -> &'static str {
    tr("perm-owner")
}
pub fn perm_group() -> &'static str {
    tr("perm-group")
}
pub fn perm_others() -> &'static str {
    tr("perm-others")
}
pub fn perm_read() -> &'static str {
    tr("perm-read")
}
pub fn perm_write() -> &'static str {
    tr("perm-write")
}
pub fn perm_execute() -> &'static str {
    tr("perm-execute")
}
pub fn perm_octal() -> &'static str {
    tr("perm-octal")
}
pub fn octal_invalid() -> &'static str {
    tr("octal-invalid")
}
pub fn apply() -> &'static str {
    tr("apply")
}
/// How many names a confirmation lists before "and N more".
pub const LISTED_NAMES: usize = 5;

/// "New folder 2".
pub fn numbered(name: &str, n: u32) -> String {
    format!("{name} {n}")
}

/// "Delete report.pdf?", "Delete 3 items?".
pub fn delete_question(names: &[&str]) -> String {
    match names {
        [one] => tr_args("delete-question-one", &[("name", (*one).into())]),
        many => tr_args("delete-question-many", &[("count", many.len().into())]),
    }
}

/// "and 4 more".
pub fn and_more(n: usize) -> String {
    tr_args("and-more", &[("count", n.into())])
}

/// "Deleting… 12 items".
pub fn deleting(done: u64) -> String {
    tr_args("deleting", &[("done", done.into())])
}

/// A delete that stopped after removing some items.
pub fn deleted_then(done: u64, error: &str) -> String {
    tr_args(
        "deleted-then",
        &[("done", done.into()), ("error", error.into())],
    )
}

// Security and credential dialogs.
pub fn host_key_title() -> &'static str {
    tr("host-key-title")
}
pub fn host_key_intro() -> &'static str {
    tr("host-key-intro")
}
pub fn trust_and_connect() -> &'static str {
    tr("trust-and-connect")
}
pub fn cert_untrusted_title() -> &'static str {
    tr("cert-untrusted-title")
}
pub fn cert_untrusted_intro() -> &'static str {
    tr("cert-untrusted-intro")
}
pub fn cert_changed_title() -> &'static str {
    tr("cert-changed-title")
}
pub fn cert_changed_intro() -> &'static str {
    tr("cert-changed-intro")
}
pub fn trust_certificate() -> &'static str {
    tr("trust-certificate")
}
pub fn label_host() -> &'static str {
    tr("label-host")
}
pub fn label_account() -> &'static str {
    tr("label-account")
}
pub fn label_key_type() -> &'static str {
    tr("label-key-type")
}
pub fn label_fingerprint() -> &'static str {
    tr("label-fingerprint")
}
pub fn label_reason() -> &'static str {
    tr("label-reason")
}
pub fn label_trusted_before() -> &'static str {
    tr("label-trusted-before")
}
pub fn label_presented_now() -> &'static str {
    tr("label-presented-now")
}
pub fn label_key_file() -> &'static str {
    tr("label-key-file")
}
pub fn password_title() -> &'static str {
    tr("password-title")
}
pub fn passphrase_title() -> &'static str {
    tr("passphrase-title")
}
pub fn field_passphrase() -> &'static str {
    tr("field-passphrase")
}
pub fn kbd_title() -> &'static str {
    tr("kbd-title")
}
pub fn save_in_keyring() -> &'static str {
    tr("save-in-keyring")
}
pub fn save_in_keyring_hint() -> &'static str {
    tr("save-in-keyring-hint")
}
pub fn connect() -> &'static str {
    tr("connect")
}
pub fn unlock() -> &'static str {
    tr("unlock")
}
pub fn continue_() -> &'static str {
    tr("continue")
}
/// Under a dialog, when others wait behind it: "1 more waiting".
pub fn dialogs_waiting(n: usize) -> String {
    tr_args("dialogs-waiting", &[("count", n.into())])
}
/// The warning of a host known with other key types.
pub fn other_key_types(types: &[String]) -> String {
    tr_args(
        "other-key-types",
        &[("types", types.join(", ").as_str().into())],
    )
}
/// Why a certificate needs the user's decision.
pub fn cert_reason(reason: &hoprs_remote::CertReason) -> String {
    use hoprs_remote::CertReason;
    match reason {
        CertReason::Expired => tr("cert-reason-expired").to_owned(),
        CertReason::NotYetValid => tr("cert-reason-not-yet-valid").to_owned(),
        CertReason::WrongName => tr("cert-reason-wrong-name").to_owned(),
        CertReason::UnknownIssuer => tr("cert-reason-unknown-issuer").to_owned(),
        CertReason::Other(text) => text.clone(),
    }
}
// Changed host key, in the tab.
pub fn host_key_changed_title() -> &'static str {
    tr("host-key-changed-title")
}
pub fn host_key_changed_body() -> &'static str {
    tr("host-key-changed-body")
}
pub fn host_key_changed_session_body() -> &'static str {
    tr("host-key-changed-session-body")
}
pub fn remove_old_entry() -> &'static str {
    tr("remove-old-entry")
}
pub fn close_and_reconnect() -> &'static str {
    tr("close-and-reconnect")
}
pub fn reading_known_hosts() -> &'static str {
    tr("reading-known-hosts")
}
pub fn removing_known_hosts() -> &'static str {
    tr("removing-known-hosts")
}
pub fn remove() -> &'static str {
    tr("remove")
}
pub fn connect_again() -> &'static str {
    tr("connect-again")
}
pub fn no_known_hosts_entry() -> &'static str {
    tr("no-known-hosts-entry")
}
/// "known_hosts line 3".
pub fn known_hosts_line(line: usize) -> String {
    tr_args(
        "known-hosts-line",
        &[("line", line.to_string().as_str().into())],
    )
}
/// The removal question: "Remove 2 lines for [h]:2222 from known_hosts? …".
pub fn remove_known_hosts_question(lines: usize, host: &str) -> String {
    tr_args(
        "remove-known-hosts-question",
        &[
            ("lines", lines.into()),
            ("lines_text", lines.to_string().as_str().into()),
            ("host", host.into()),
        ],
    )
}
/// The warning for lines that also name other hosts.
pub fn also_covers_other_hosts(lines: &[usize]) -> String {
    let list: Vec<String> = lines.iter().map(ToString::to_string).collect();
    match lines {
        [line] => tr_args(
            "also-covers-one-host",
            &[("line", line.to_string().as_str().into())],
        ),
        _ => tr_args(
            "also-covers-other-hosts",
            &[("lines", list.join(", ").as_str().into())],
        ),
    }
}
// Import from FileZilla.
pub fn import_title() -> &'static str {
    tr("import-title")
}
pub fn import_step_file() -> &'static str {
    tr("import-step-file")
}
pub fn import_step_folders() -> &'static str {
    tr("import-step-folders")
}
pub fn import_step_report() -> &'static str {
    tr("import-step-report")
}
pub fn import_file_label() -> &'static str {
    tr("import-file-label")
}
pub fn import_file_hint() -> &'static str {
    tr("import-file-hint")
}
pub fn import_path_required() -> &'static str {
    tr("import-path-required")
}
pub fn choose_file() -> &'static str {
    tr("choose-file")
}
pub fn next() -> &'static str {
    tr("next")
}
pub fn back() -> &'static str {
    tr("back")
}
pub fn import_folders_question() -> &'static str {
    tr("import-folders-question")
}
pub fn import_folders_hint() -> &'static str {
    tr("import-folders-hint")
}
pub fn import_folders_yes() -> &'static str {
    tr("import-folders-yes")
}
pub fn import_folders_no() -> &'static str {
    tr("import-folders-no")
}
pub fn import_running() -> &'static str {
    tr("import-running")
}
pub fn import_imported() -> &'static str {
    tr("import-imported")
}
pub fn import_skipped() -> &'static str {
    tr("import-skipped")
}
pub fn import_workspaces_created() -> &'static str {
    tr("import-workspaces-created")
}
pub fn import_workspaces_not_created() -> &'static str {
    tr("import-workspaces-not-created")
}
pub fn import_failures() -> &'static str {
    tr("import-failures")
}
pub fn import_none() -> &'static str {
    tr("import-none")
}
pub fn import_failed_keep_source() -> &'static str {
    tr("import-failed-keep-source")
}
pub fn import_passwords_keep_source() -> &'static str {
    tr("import-passwords-keep-source")
}
pub fn delete_sitemanager() -> &'static str {
    tr("delete-sitemanager")
}
pub fn delete_sitemanager_confirm() -> &'static str {
    tr("delete-sitemanager-confirm")
}
pub fn deleting_sitemanager() -> &'static str {
    tr("deleting-sitemanager")
}
pub fn sitemanager_deleted() -> &'static str {
    tr("sitemanager-deleted")
}
pub fn done() -> &'static str {
    tr("done")
}
pub fn pick_sitemanager() -> &'static str {
    tr("pick-sitemanager")
}
pub fn pick_filter_xml() -> &'static str {
    tr("pick-filter-xml")
}
// Settings.
pub fn settings_general() -> &'static str {
    tr("settings-general")
}
pub fn settings_general_description() -> &'static str {
    tr("settings-general-description")
}
pub fn settings_transfers_description() -> &'static str {
    tr("settings-transfers-description")
}
pub fn settings_compare_description() -> &'static str {
    tr("settings-compare-description")
}
pub fn settings_editing_description() -> &'static str {
    tr("settings-editing-description")
}
pub fn settings_terminal_description() -> &'static str {
    tr("settings-terminal-description")
}
pub fn settings_backup_description() -> &'static str {
    tr("settings-backup-description")
}
pub fn settings_transfers() -> &'static str {
    tr("settings-transfers")
}
pub fn settings_workers() -> &'static str {
    tr("settings-workers")
}
pub fn settings_workers_hint() -> &'static str {
    tr("settings-workers-hint")
}
pub fn settings_policy() -> &'static str {
    tr("settings-policy")
}
pub fn settings_policy_hint() -> &'static str {
    tr("settings-policy-hint")
}
pub fn settings_keepalive() -> &'static str {
    tr("settings-keepalive")
}
pub fn settings_keepalive_hint() -> &'static str {
    tr("settings-keepalive-hint")
}
/// A keepalive interval: "30 s".
pub fn seconds(secs: u64) -> String {
    tr_args("seconds", &[("secs", secs.into())])
}
pub fn settings_unreadable() -> &'static str {
    tr("settings-unreadable")
}
pub fn settings_save_failed() -> &'static str {
    tr("settings-save-failed")
}
pub fn settings_terminal() -> &'static str {
    tr("settings-terminal")
}
pub fn settings_font_size() -> &'static str {
    tr("settings-font-size")
}
pub fn settings_scrollback() -> &'static str {
    tr("settings-scrollback")
}
pub fn settings_scrollback_hint() -> &'static str {
    tr("settings-scrollback-hint")
}
pub fn settings_theme() -> &'static str {
    tr("settings-theme")
}
pub fn settings_language() -> &'static str {
    tr("settings-language")
}
pub fn language_system() -> &'static str {
    tr("language-system")
}
pub fn language_english() -> &'static str {
    tr("language-english")
}
pub fn language_pt_br() -> &'static str {
    tr("language-pt-br")
}
pub fn language_tr() -> &'static str {
    tr("language-tr")
}
pub fn theme_hoprs_dark() -> &'static str {
    tr("theme-hoprs-dark")
}
pub fn policy_ask() -> &'static str {
    tr("settings-policy-ask")
}
pub fn policy_overwrite() -> &'static str {
    tr("settings-policy-overwrite")
}
pub fn policy_skip() -> &'static str {
    tr("settings-policy-skip")
}
pub fn policy_overwrite_if_newer() -> &'static str {
    tr("settings-policy-overwrite-if-newer")
}
pub fn policy_resume() -> &'static str {
    tr("settings-policy-resume")
}
pub fn decrease() -> &'static str {
    tr("decrease")
}
pub fn increase() -> &'static str {
    tr("increase")
}
// Backup.
pub fn backup() -> &'static str {
    tr("backup")
}
pub fn backup_hint() -> &'static str {
    tr("backup-hint")
}
pub fn export_backup() -> &'static str {
    tr("export-backup")
}
pub fn restore_backup() -> &'static str {
    tr("restore-backup")
}
pub fn backup_file() -> &'static str {
    tr("backup-file")
}
pub fn backup_password() -> &'static str {
    tr("backup-password")
}
pub fn backup_password_confirm() -> &'static str {
    tr("backup-password-confirm")
}
pub fn backup_path_required() -> &'static str {
    tr("backup-path-required")
}
pub fn backup_password_required() -> &'static str {
    tr("backup-password-required")
}
pub fn backup_passwords_differ() -> &'static str {
    tr("backup-passwords-differ")
}
pub fn drive_name_required() -> &'static str {
    tr("drive-name-required")
}
pub fn drive_page_connected() -> &'static str {
    tr("drive-page-connected")
}
pub fn drive_page_failed() -> &'static str {
    tr("drive-page-failed")
}
pub fn export_destination() -> &'static str {
    tr("export-destination")
}
pub fn export_destination_file() -> &'static str {
    tr("export-destination-file")
}
pub fn export_destination_drive() -> &'static str {
    tr("export-destination-drive")
}
pub fn drive_connect() -> &'static str {
    tr("drive-connect")
}
pub fn drive_account() -> &'static str {
    tr("drive-account")
}
pub fn drive_connected() -> &'static str {
    tr("drive-connected")
}
pub fn drive_disconnect() -> &'static str {
    tr("drive-disconnect")
}
pub fn drive_file_name() -> &'static str {
    tr("drive-file-name")
}
pub fn drive_waiting() -> &'static str {
    tr("drive-waiting")
}
pub fn drive_copy_link() -> &'static str {
    tr("drive-copy-link")
}
pub fn drive_browser_hint() -> &'static str {
    tr("drive-browser-hint")
}
pub fn drive_session_only() -> &'static str {
    tr("drive-session-only")
}
pub fn drive_uploading() -> &'static str {
    tr("drive-uploading")
}
pub fn drive_uploaded(path: &str) -> String {
    tr_args("drive-uploaded", &[("path", path.into())])
}
pub fn drive_open() -> &'static str {
    tr("drive-open")
}
pub fn drive_save_local() -> &'static str {
    tr("drive-save-local")
}
pub fn drive_saved_local(path: &str) -> String {
    tr_args("drive-saved-local", &[("path", path.into())])
}
pub fn drive_settings_line() -> &'static str {
    tr("drive-settings-line")
}
pub fn export() -> &'static str {
    tr("export")
}
pub fn export_overwrite_confirm() -> &'static str {
    tr("export-overwrite-confirm")
}
pub fn restore() -> &'static str {
    tr("restore")
}
pub fn exporting() -> &'static str {
    tr("exporting")
}
pub fn restoring() -> &'static str {
    tr("restoring")
}
pub fn restore_mode() -> &'static str {
    tr("restore-mode")
}
pub fn restore_merge() -> &'static str {
    tr("restore-merge")
}
pub fn restore_merge_hint() -> &'static str {
    tr("restore-merge-hint")
}
pub fn restore_replace() -> &'static str {
    tr("restore-replace")
}
pub fn restore_replace_hint() -> &'static str {
    tr("restore-replace-hint")
}
pub fn restore_replace_confirm() -> &'static str {
    tr("restore-replace-confirm")
}
pub fn replace() -> &'static str {
    tr("replace")
}
pub fn export_failed() -> &'static str {
    tr("export-failed")
}
pub fn restore_failed() -> &'static str {
    tr("restore-failed")
}
pub fn backup_secret_errors() -> &'static str {
    tr("backup-secret-errors")
}
pub fn backup_missing_secrets() -> &'static str {
    tr("backup-missing-secrets")
}
pub fn backup_skipped_unreadable() -> &'static str {
    tr("backup-skipped-unreadable")
}
pub fn backup_skipped_sites() -> &'static str {
    tr("backup-skipped-sites")
}
pub fn backup_skipped_workspaces() -> &'static str {
    tr("backup-skipped-workspaces")
}
pub fn pick_export() -> &'static str {
    tr("pick-export")
}
pub fn pick_restore() -> &'static str {
    tr("pick-restore")
}
/// "Exported 12 sites, 3 workspaces and 9 passwords."
pub fn export_done(sites: usize, workspaces: usize, secrets: usize) -> String {
    tr_args(
        "export-done",
        &[
            ("sites", site_count(sites).as_str().into()),
            ("workspaces", workspace_count(workspaces).as_str().into()),
            ("secrets", password_count(secrets).as_str().into()),
        ],
    )
}

/// `Export "Cliente Acme" (site)`.
pub fn export_title_site(name: &str) -> String {
    tr_args("export-title-site", &[("name", name.into())])
}
/// `Export "Cliente Acme" (workspace, 4 sites)`.
pub fn export_title_workspace(name: &str, sites: usize) -> String {
    tr_args(
        "export-title-workspace",
        &[
            ("name", name.into()),
            ("sites", site_count(sites).as_str().into()),
        ],
    )
}
pub fn export_ellipsis() -> &'static str {
    tr("export-ellipsis")
}
pub fn export_everything() -> &'static str {
    tr("export-everything")
}
/// `Export everything (12 sites, 3 workspaces)`.
pub fn export_title_all(sites: usize, workspaces: usize) -> String {
    tr_args(
        "export-title-all",
        &[
            ("sites", site_count(sites).as_str().into()),
            ("workspaces", workspace_count(workspaces).as_str().into()),
        ],
    )
}
pub fn export_password_hint() -> &'static str {
    tr("export-password-hint")
}
pub fn export_partial_note() -> &'static str {
    tr("export-partial-note")
}

/// A number of workspaces: "1 workspace", "3 workspaces".
fn workspace_count(n: usize) -> String {
    tr_args("workspace-count", &[("count", n.into())])
}

/// A number of saved passwords: "1 password", "3 passwords".
fn password_count(n: usize) -> String {
    tr_args("password-count", &[("count", n.into())])
}
/// "Restored 2 sites, 1 workspace and 2 passwords."
pub fn restore_done(sites: usize, workspaces: usize, secrets: usize) -> String {
    tr_args(
        "restore-done",
        &[
            ("sites", site_count(sites).as_str().into()),
            ("workspaces", workspace_count(workspaces).as_str().into()),
            ("secrets", password_count(secrets).as_str().into()),
        ],
    )
}
/// A report section's head: "Imported  4".
pub fn section_count(title: &str, count: usize) -> String {
    format!("{title}  {count}")
}
// Remote editing.
/// The Edit action (remote pane).
pub fn edit_file() -> &'static str {
    tr("edit-file")
}
pub fn edit_no_folders() -> &'static str {
    tr("edit-no-folders")
}
pub fn edit_no_links() -> &'static str {
    tr("edit-no-links")
}
pub fn edit_too_large() -> &'static str {
    tr("edit-too-large")
}
pub fn waiting_for_connection() -> &'static str {
    tr("waiting-for-connection")
}
pub fn remote_may_be_truncated() -> &'static str {
    tr("remote-may-be-truncated")
}
pub fn changed_on_server() -> &'static str {
    tr("changed-on-server")
}
pub fn keep_remote() -> &'static str {
    tr("keep-remote")
}
pub fn save_a_copy() -> &'static str {
    tr("save-a-copy")
}
pub fn reload_in_editor() -> &'static str {
    tr("reload-in-editor")
}
pub fn upload_and_close() -> &'static str {
    tr("upload-and-close")
}
pub fn close_keep_copies() -> &'static str {
    tr("close-keep-copies")
}
pub fn upload_and_quit() -> &'static str {
    tr("upload-and-quit")
}
pub fn quit_keep_copies() -> &'static str {
    tr("quit-keep-copies")
}
pub fn checking_edits() -> &'static str {
    tr("checking-edits")
}
pub fn stop_keep_copies() -> &'static str {
    tr("stop-keep-copies")
}
/// While "Upload" runs (an upload under way has no bound but its protocol's timeouts).
pub fn uploading_edits() -> &'static str {
    tr("uploading-edits")
}
/// The kept notice, before a remote path: "Server copy of".
pub fn server_copy_of() -> &'static str {
    tr("server-copy-of")
}
/// …after it.
pub fn may_be_incomplete() -> &'static str {
    tr("may-be-incomplete")
}
/// The kept notice after a close that did not ask (the site moved, "Close and
/// reconnect"): the editor's file is no longer linked to the server.
pub fn kept_saves_stopped() -> &'static str {
    tr("kept-saves-stopped")
}
pub fn edit_one_file() -> &'static str {
    tr("edit-one-file")
}
pub fn uploaded() -> &'static str {
    tr("uploaded")
}
pub fn editing() -> &'static str {
    tr("editing")
}
pub fn opening() -> &'static str {
    tr("opening")
}
pub fn close_workspace_edits_title() -> &'static str {
    tr("close-workspace-edits-title")
}
pub fn keeping_copies() -> &'static str {
    tr("keeping-copies")
}
pub fn close_edits_title() -> &'static str {
    tr("close-edits-title")
}
pub fn quit_edits_title() -> &'static str {
    tr("quit-edits-title")
}
pub fn quit_edits_hint() -> &'static str {
    tr("quit-edits-hint")
}
pub fn edit_unsent() -> &'static str {
    tr("edit-unsent")
}
pub fn edit_conflict() -> &'static str {
    tr("edit-conflict")
}
pub fn editor_command() -> &'static str {
    tr("editor-command")
}
/// Windows' example names `code.cmd`: the command runs without a shell, which does not
/// find `.cmd` shims from a bare name.
pub fn editor_command_hint() -> &'static str {
    if cfg!(windows) {
        tr("editor-command-hint-windows")
    } else if cfg!(target_os = "macos") {
        tr("editor-command-hint-macos")
    } else {
        tr("editor-command-hint")
    }
}
/// Next to the editor field when it holds `system`.
pub fn editor_system_default() -> &'static str {
    tr("editor-system-default")
}
pub fn editor_command_empty() -> &'static str {
    tr("editor-command-empty")
}
pub fn settings_editing() -> &'static str {
    tr("settings-editing")
}
pub fn settings_compare() -> &'static str {
    tr("settings-compare")
}
pub fn settings_compare_auto() -> &'static str {
    tr("settings-compare-auto")
}
pub fn settings_compare_ignore() -> &'static str {
    tr("settings-compare-ignore")
}
pub fn settings_compare_ignore_hint() -> &'static str {
    tr("settings-compare-ignore-hint")
}
/// "Upload failed: Permission denied".
pub fn upload_failed(reason: &str) -> String {
    tr_args("upload-failed", &[("reason", reason.into())])
}
/// "Your version was saved to <path>".
pub fn your_version_at(path: &str) -> String {
    tr_args("your-version-at", &[("path", path.into())])
}
/// Where "keep" puts the copies: "Unsent changes are copied to <dir>".
pub fn copies_go_to(dir: &str) -> String {
    tr_args("copies-go-to", &[("dir", dir.into())])
}
/// "2 files could not be uploaded".
pub fn not_uploaded(n: usize) -> String {
    tr_args("not-uploaded", &[("count", n.into())])
}
/// The persistent notice after a close kept unsent edits: "Kept 2 unsent edits as
/// recovered copies in" (the folder follows, in mono).
pub fn kept_notice(n: usize) -> String {
    tr_args("kept-notice", &[("count", n.into())])
}
/// The delete confirmation of a site with files open for editing.
pub fn delete_site_edits(n: usize) -> String {
    tr_args("delete-site-edits", &[("count", n.into())])
}

// The folder compare panel.
pub fn compare_button() -> &'static str {
    tr("compare-button")
}
pub fn compare_title() -> &'static str {
    tr("compare-title")
}
pub fn compare_auto() -> &'static str {
    tr("compare-auto")
}
pub fn compare_close() -> &'static str {
    tr("compare-close")
}
pub fn compare_collapse() -> &'static str {
    tr("compare-collapse")
}
pub fn compare_expand() -> &'static str {
    tr("compare-expand")
}
pub fn compare_local_newer() -> &'static str {
    tr("compare-local-newer")
}
pub fn compare_remote_newer() -> &'static str {
    tr("compare-remote-newer")
}
pub fn compare_size_differs() -> &'static str {
    tr("compare-size-differs")
}
pub fn compare_kind_differs() -> &'static str {
    tr("compare-kind-differs")
}
pub fn compare_queued() -> &'static str {
    tr("compare-queued")
}
pub fn compare_upload_selected() -> &'static str {
    tr("compare-upload-selected")
}
pub fn compare_download_selected() -> &'static str {
    tr("compare-download-selected")
}
pub fn compare_continue() -> &'static str {
    tr("compare-continue")
}
pub fn compare_stop() -> &'static str {
    tr("compare-stop")
}
pub fn compare_partial() -> &'static str {
    tr("compare-partial")
}
pub fn compare_again() -> &'static str {
    tr("compare-again")
}
pub fn compare_summary_only_here(count: usize) -> String {
    tr_args("compare-summary-only-here", &[("count", count.into())])
}
pub fn compare_summary_only_server(count: usize) -> String {
    tr_args("compare-summary-only-server", &[("count", count.into())])
}
pub fn compare_summary_different(count: usize) -> String {
    tr_args("compare-summary-different", &[("count", count.into())])
}
pub fn compare_group_only_local(count: usize) -> String {
    tr_args("compare-group-only-local", &[("count", count.into())])
}
pub fn compare_group_only_remote(count: usize) -> String {
    tr_args("compare-group-only-remote", &[("count", count.into())])
}
pub fn compare_group_different(count: usize) -> String {
    tr_args("compare-group-different", &[("count", count.into())])
}
pub fn compare_group_unreadable(count: usize) -> String {
    tr_args("compare-group-unreadable", &[("count", count.into())])
}
pub fn compare_identical(count: usize) -> String {
    tr_args("compare-identical", &[("count", count.into())])
}
pub fn compare_all_identical(count: usize) -> String {
    tr_args("compare-all-identical", &[("count", count.into())])
}
pub fn compare_running(count: usize) -> String {
    tr_args("compare-running", &[("count", count.into())])
}
pub fn compare_paused(count: usize) -> String {
    tr_args("compare-paused", &[("count", count.into())])
}
pub fn compare_unreadable(error: &str) -> String {
    tr_args("compare-unreadable", &[("error", error.into())])
}
pub fn compare_stopped(error: &str) -> String {
    tr_args("compare-stopped", &[("error", error.into())])
}
pub fn compare_tip_local(when: &str) -> String {
    tr_args("compare-tip-local", &[("when", when.into())])
}
pub fn compare_tip_remote(when: &str) -> String {
    tr_args("compare-tip-remote", &[("when", when.into())])
}

pub fn compare_roots(local: &str, remote: &str) -> String {
    tr_args(
        "compare-roots",
        &[("local", local.into()), ("remote", remote.into())],
    )
}

pub fn ellipsis() -> &'static str {
    tr("compare-ellipsis")
}

#[cfg(test)]
mod tests {
    use crate::i18n::{Catalog, Language};
    use hoprs_core::{Error, ErrorKind};

    /// Views must show a core `Error` through `strings::error`, never its raw `message`.
    #[test]
    fn views_show_errors_through_strings_error() {
        fn scan(dir: &std::path::Path, hits: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    scan(&path, hits);
                    continue;
                }
                let name = path.file_name().unwrap().to_string_lossy().into_owned();
                // The log panel stays English; strings.rs holds this very test.
                if !name.ends_with(".rs") || name == "log.rs" || name == "strings.rs" {
                    continue;
                }
                let source = std::fs::read_to_string(&path).unwrap();
                for (n, line) in source.lines().enumerate() {
                    // `banner.message` is already composed text, not a core error.
                    if line.contains(".message") && !line.contains("banner.message") {
                        hits.push(format!("{name}:{}: {}", n + 1, line.trim()));
                    }
                }
            }
        }
        let mut hits = Vec::new();
        scan(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/ui"),
            &mut hits,
        );
        assert!(hits.is_empty(), "route through strings::error: {hits:#?}");
    }

    #[test]
    fn error_text_uses_translation_then_message() {
        let en = "error-dns = Cannot resolve { $host }\nerror-bare = Bare\n";
        let pt = "error-dns = Nao foi possivel resolver { $host }\nerror-bare = Simples\n";
        let c = Catalog::from_sources(Language::PtBr, en, Some(pt));
        let e = Error::coded(ErrorKind::Network, "dns", "Cannot resolve h").arg("host", "h");
        assert_eq!(super::error_in(&c, &e), "Nao foi possivel resolver h");
        let en_only = Catalog::from_sources(Language::En, en, None);
        assert_eq!(super::error_in(&en_only, &e), "Cannot resolve h");
        // A missing argument: the English message, never a `{$host}`.
        let missing = Error::coded(ErrorKind::Network, "dns", "Cannot resolve h");
        assert_eq!(super::error_in(&c, &missing), "Cannot resolve h");
        // Unknown code and no code.
        let unknown = Error::coded(ErrorKind::Io, "nothing", "Plain English");
        assert_eq!(super::error_in(&c, &unknown), "Plain English");
        assert_eq!(
            super::error_in(&c, &Error::new(ErrorKind::Io, "Uncoded")),
            "Uncoded"
        );
        assert_eq!(
            super::error_in(&c, &Error::coded(ErrorKind::Io, "bare", "Bare")),
            "Simples"
        );
        // The active-language wrapper uses English here.
        assert_eq!(super::error(&unknown), "Plain English");
    }

    fn pt(key: &str, args: &[(&str, crate::i18n::Arg)]) -> String {
        crate::i18n::catalog_for(Language::PtBr).tr_args(key, args)
    }

    #[test]
    fn identifier_numbers_are_not_grouped() {
        assert_eq!(super::known_hosts_line(1234), "known_hosts line 1234");
        assert_eq!(
            pt("known-hosts-line", &[("line", "1234".into())]),
            "Linha 1234 do known_hosts"
        );
        assert_eq!(
            super::shell_exited(Some(3221225477)),
            "Shell exited (code 3221225477)"
        );
        assert!(super::remove_known_hosts_question(1234, "h").contains("Remove 1234 lines"));
        assert!(super::also_covers_other_hosts(&[1234]).starts_with("Line 1234 "));
        assert!(super::unreadable_name(None, "f", 1233).contains("1234"));
    }

    #[test]
    fn pt_br_site_count_forms() {
        for (n, want) in [(0usize, "0 sites"), (1, "1 site"), (2, "2 sites")] {
            assert_eq!(pt("site-count", &[("count", n.into())]), want);
        }
    }

    #[test]
    fn pt_br_workspace_site_count_partial() {
        let partial = |present: usize, total: usize| {
            pt(
                "workspace-site-count-partial",
                &[("present", present.into()), ("total", total.into())],
            )
        };
        assert_eq!(partial(0, 1), "0 de 1 site");
        assert_eq!(partial(1, 2), "1 de 2 sites");
        assert_eq!(partial(2, 3), "2 de 3 sites");
    }

    #[test]
    fn pt_br_unreadable_name_and_folder_and_shell() {
        assert_eq!(
            pt(
                "unreadable-name-fallback",
                &[("number", 3usize.into()), ("file", "sites.toml".into())]
            ),
            "Entrada 3 em sites.toml"
        );
        assert_eq!(
            pt("cannot-open-folder", &[("reason", "negado".into())]),
            "Não foi possível abrir a pasta: negado"
        );
        assert_eq!(
            pt("shell-exited-code", &[("code", 130u32.into())]),
            "Shell encerrado (código 130)"
        );
        assert_eq!(pt("shell-exited", &[]), "Shell encerrado");
    }

    #[test]
    fn pt_br_log_lines_and_total_workers() {
        for (n, want) in [(0usize, "0 linhas"), (1, "1 linha"), (2, "2 linhas")] {
            assert_eq!(pt("log-lines", &[("count", n.into())]), want);
        }
        for (n, want) in [
            (0usize, "no total, 0 workers"),
            (1, "no total, 1 worker"),
            (4, "no total, 4 workers"),
        ] {
            assert_eq!(pt("total-workers", &[("workers", n.into())]), want);
        }
    }

    #[test]
    fn total_workers_keeps_its_leading_space() {
        assert_eq!(super::total_workers(1), " total, 1 worker");
        assert_eq!(super::total_workers(4), " total, 4 workers");
    }

    #[test]
    fn pt_br_session_policy_and_queue_texts() {
        assert_eq!(
            pt("session-policy", &[("name", "sobrescrever".into())]),
            "Conflitos: sobrescrever (nesta sessão)"
        );
        assert_eq!(
            pt("session-policy-tag", &[("name", "se mais novo".into())]),
            "se mais novo: sessão"
        );
        assert!(
            pt("queue-unavailable", &[("reason", "disco cheio".into())]).contains("disco cheio")
        );
        assert_eq!(
            pt("saved-folder-gone", &[("saved", "/srv".into())]),
            "A pasta salva /srv não existe mais; mostrando a pasta mais próxima."
        );
    }

    #[test]
    fn pt_br_skip_reasons_translate_and_unknown_ones_pass_through() {
        let cat = Catalog::new(Language::PtBr);
        assert_eq!(cat.tr("skip-exists"), "já existe");
        assert_eq!(cat.tr("skip-not-newer"), "não é mais recente");
        assert_eq!(cat.tr("skip-already-complete"), "já está completo");
        assert_eq!(
            pt("skipped", &[("reason", "já existe".into())]),
            "Ignorada: já existe"
        );
        // English process language: known reasons read as before, others unchanged.
        assert_eq!(cat.tr("skip-by-user"), "pulada por você");
        assert_eq!(cat.tr("skip-symlink"), "links simbólicos não são seguidos");
        assert_eq!(super::skip_reason("skipped by user"), "skipped by user");
        assert_eq!(
            super::skip_reason("symlinks are not followed"),
            "symlinks are not followed"
        );
        assert_eq!(super::skip_reason("exists"), "exists");
        assert_eq!(super::skip_reason("not newer"), "not newer");
        assert_eq!(super::skip_reason("already complete"), "already complete");
        assert_eq!(super::skip_reason("disk full"), "disk full");
        assert_eq!(super::skipped("disk full"), "Skipped: disk full");
    }

    #[test]
    fn pt_br_status_and_auth_texts() {
        assert_eq!(
            pt(
                "reconnecting",
                &[("attempt", 2u32.into()), ("of", 5u32.into())]
            ),
            "Reconectando (2/5)"
        );
        assert_eq!(
            pt("disconnected", &[("reason", "queda".into())]),
            "Desconectado — queda"
        );
        assert_eq!(
            pt("auth-agent", &[("key_type", "ed25519".into())]),
            "ed25519 via ssh-agent"
        );
        assert_eq!(
            pt("auth-key-file", &[("key_type", "rsa".into())]),
            "chave rsa"
        );
        assert_eq!(
            pt(
                "workspace-status",
                &[
                    ("connected", 2usize.into()),
                    ("sites", "3 sites".into()),
                    ("name", "Loja".into())
                ]
            ),
            "2 de 3 sites em Loja"
        );
    }

    #[test]
    fn pt_br_delete_texts() {
        assert_eq!(
            pt("delete-question-one", &[("name", "a.txt".into())]),
            "Excluir a.txt?"
        );
        assert_eq!(
            pt("delete-question-many", &[("count", 3usize.into())]),
            "Excluir 3 itens?"
        );
        assert_eq!(pt("and-more", &[("count", 4usize.into())]), "e mais 4");
        for (n, want) in [
            (0u64, "Excluindo…"),
            (1, "Excluindo… 1 item"),
            (12, "Excluindo… 12 itens"),
        ] {
            assert_eq!(pt("deleting", &[("done", n.into())]), want);
        }
        let then = |n: u64| {
            pt(
                "deleted-then",
                &[("done", n.into()), ("error", "negado".into())],
            )
        };
        assert_eq!(then(0), "0 itens excluídos; depois: negado");
        assert_eq!(then(1), "1 item excluído; depois: negado");
        assert_eq!(then(2), "2 itens excluídos; depois: negado");
    }

    #[test]
    fn pt_br_size_units_and_months() {
        let cat = Catalog::new(Language::PtBr);
        assert_eq!(cat.tr("size-unit-kb"), "KB");
        let months: Vec<_> = (1..=12).map(|n| cat.tr(&format!("month-{n}"))).collect();
        assert_eq!(
            months.join(" "),
            "jan fev mar abr mai jun jul ago set out nov dez"
        );
    }

    #[test]
    fn pt_br_known_hosts_texts() {
        assert_eq!(
            pt("known-hosts-line", &[("line", 3usize.into())]),
            "Linha 3 do known_hosts"
        );
        for (n, want) in [
            (
                1usize,
                "Remover 1 linha de [h]:2222 do known_hosts? Na próxima conexão, o hoprs pede que você confie na nova chave.",
            ),
            (
                2,
                "Remover 2 linhas de [h]:2222 do known_hosts? Na próxima conexão, o hoprs pede que você confie na nova chave.",
            ),
        ] {
            assert_eq!(
                pt(
                    "remove-known-hosts-question",
                    &[
                        ("lines", n.into()),
                        ("lines_text", n.to_string().as_str().into()),
                        ("host", "[h]:2222".into())
                    ]
                ),
                want
            );
        }
        assert!(
            pt("also-covers-one-host", &[("line", 4usize.into())]).starts_with("A linha 4 também")
        );
        assert!(
            pt("also-covers-other-hosts", &[("lines", "3, 5".into())])
                .starts_with("As linhas 3, 5 também")
        );
        assert_eq!(
            pt("dialogs-waiting", &[("count", 1usize.into())]),
            "Mais 1 na fila"
        );
    }

    #[test]
    fn pt_br_counts_and_backup_reports() {
        for (n, want) in [
            (0usize, "0 workspaces"),
            (1, "1 workspace"),
            (2, "2 workspaces"),
        ] {
            assert_eq!(pt("workspace-count", &[("count", n.into())]), want);
        }
        for (n, want) in [(0usize, "0 senhas"), (1, "1 senha"), (2, "2 senhas")] {
            assert_eq!(pt("password-count", &[("count", n.into())]), want);
        }
        assert_eq!(
            pt(
                "export-done",
                &[
                    ("sites", "12 sites".into()),
                    ("workspaces", "3 workspaces".into()),
                    ("secrets", "9 senhas".into())
                ]
            ),
            "Exportados 12 sites, 3 workspaces e 9 senhas."
        );
        assert_eq!(
            pt(
                "restore-done",
                &[
                    ("sites", "2 sites".into()),
                    ("workspaces", "1 workspace".into()),
                    ("secrets", "2 senhas".into())
                ]
            ),
            "Restaurados 2 sites, 1 workspace e 2 senhas."
        );
        assert_eq!(pt("seconds", &[("secs", 30u64.into())]), "30 s");
    }

    #[test]
    fn english_counts_and_backup_reports() {
        assert_eq!(
            super::export_done(12, 3, 9),
            "Exported 12 sites, 3 workspaces and 9 passwords."
        );
        assert_eq!(
            super::restore_done(2, 1, 2),
            "Restored 2 sites, 1 workspace and 2 passwords."
        );
        assert_eq!(
            super::restore_done(0, 1, 1),
            "Restored 0 sites, 1 workspace and 1 password."
        );
        assert_eq!(super::seconds(30), "30 s");
        assert_eq!(
            super::remove_known_hosts_question(1, "h"),
            "Remove 1 line for h from known_hosts? The next connect asks you to trust the new key."
        );
        assert_eq!(
            super::remove_known_hosts_question(2, "h"),
            "Remove 2 lines for h from known_hosts? The next connect asks you to trust the new key."
        );
        assert_eq!(
            super::also_covers_other_hosts(&[3, 5]),
            "Lines 3, 5 also cover other hosts (patterns or lists of names): removing them forgets these keys for them too."
        );
        assert_eq!(
            super::cert_reason(&hoprs_remote::CertReason::Expired),
            "Expired"
        );
        assert_eq!(
            super::cert_reason(&hoprs_remote::CertReason::Other("x".into())),
            "x"
        );
    }

    #[test]
    fn pt_br_remote_editing_texts() {
        let n = |key: &str, n: usize| pt(key, &[("count", n.into())]);
        assert_eq!(
            n("not-uploaded", 0),
            "Não foi possível fazer upload de 0 arquivos"
        );
        assert_eq!(
            n("not-uploaded", 1),
            "Não foi possível fazer upload de 1 arquivo"
        );
        assert_eq!(
            n("not-uploaded", 2),
            "Não foi possível fazer upload de 2 arquivos"
        );
        assert_eq!(
            n("kept-notice", 0),
            "Mantidas 0 edições não enviadas como cópias recuperadas em"
        );
        assert_eq!(
            n("kept-notice", 1),
            "Mantida 1 edição não enviada como cópia recuperada em"
        );
        assert_eq!(
            n("kept-notice", 3),
            "Mantidas 3 edições não enviadas como cópias recuperadas em"
        );
        assert!(n("delete-site-edits", 0).starts_with("0 arquivos deste site estão"));
        assert!(n("delete-site-edits", 1).starts_with("1 arquivo deste site está"));
        assert!(n("delete-site-edits", 2).starts_with("2 arquivos deste site estão"));
        assert_eq!(
            pt("upload-failed", &[("reason", "negado".into())]),
            "Falha no upload: negado"
        );
        assert_eq!(
            pt("your-version-at", &[("path", "/a/b".into())]),
            "Sua versão foi salva em /a/b"
        );
        assert_eq!(
            pt("copies-go-to", &[("dir", "/c".into())]),
            "As alterações não enviadas são copiadas para /c"
        );
        let cat = Catalog::new(Language::PtBr);
        let base = "system abre os arquivos remotos no editor padrão do sistema. Ou digite um comando; o caminho do arquivo é adicionado no fim. Por exemplo: ";
        for (key, example) in [
            ("editor-command-hint", "gedit, code --wait."),
            (
                "editor-command-hint-macos",
                "open -W -a \"Visual Studio Code\" (um caminho completo também funciona).",
            ),
            (
                "editor-command-hint-windows",
                "notepad++.exe, code.cmd --wait.",
            ),
        ] {
            assert_eq!(cat.tr(key), format!("{base}{example}"), "{key}");
        }
    }

    #[test]
    fn english_remote_editing_counts() {
        assert_eq!(super::not_uploaded(1), "1 file could not be uploaded");
        assert_eq!(super::not_uploaded(2), "2 files could not be uploaded");
        assert_eq!(
            super::kept_notice(1),
            "Kept 1 unsent edit as a recovered copy in"
        );
        assert_eq!(
            super::kept_notice(2),
            "Kept 2 unsent edits as recovered copies in"
        );
        assert!(super::delete_site_edits(1).starts_with("1 file of this site is open"));
        assert!(super::delete_site_edits(2).starts_with("2 files of this site are open"));
        assert_eq!(super::dialogs_waiting(1), "1 more waiting");
        assert!(super::editor_command_hint().contains("For example: "));
    }
}
