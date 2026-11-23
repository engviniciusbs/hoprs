# Partial export checklist (Plan 16, 1.6.0)

Automated rows first, then the steps that need a person on a real display and a second machine.

## Scopes, the export screen and partial files (2026-11-22)

| Check | Result |
| --- | --- |
| A full export is written as before: version 1, no `partial` marker | Pass: `scope_all_writes_version_1_without_the_marker` |
| A site export holds that site only | Pass: `scope_site_exports_that_site_only` |
| A workspace export holds the workspace and its readable sites; missing and unreadable members are trimmed | Pass: `scope_workspace_trims_missing_and_unreadable_members` |
| An unknown site or workspace is "not found" | Pass: `scope_of_unknown_site_or_workspace_is_not_found` |
| A partial file restores with Merge | Pass: `partial_file_merges` |
| A partial file refuses Replace and changes nothing; either marker (header version 2, `"partial": true`) alone refuses it | Pass: `partial_file_refuses_replace_and_changes_nothing`, `either_marker_alone_refuses_replace` |
| One site exported through the backend restores as one site; the full backup round trip still works | Pass: `export_of_one_site_restores_as_one_site`, `backup_export_and_restore` |
| The screen opens for each scope (site, workspace, everything), with the right title, and does nothing for a target that is gone | Pass: `open_shows_the_form_for_each_scope`, `open_for_a_missing_target_does_nothing`, `export_title_per_scope`, `title_per_scope` |
| The File field starts with the default name (`hoprs-<slug>-<date>.hoprs`, `hoprs-backup-<date>.hoprs`) | Pass: `flow_open_prefills_the_file`, `slug_rules`, `default_file_names` |
| Validation runs in order; a typed path that exists asks before overwriting; a picked path skips the question | Pass: `validation_in_order`, `typed_path_is_checked_then_asks_before_overwriting`, `picked_path_skips_the_question` |
| The note for partial scopes is shown only for a site or a workspace | Pass: `partial_note_only_for_partial_scopes` |
| The password leaves the form with the effect and is forgotten | Pass: `password_leaves_the_form_with_the_effect` |
| One export at a time, and none during a restore; the result shows the report or the error inline | Pass: `one_export_at_a_time_and_not_during_a_restore`, `result_shows_report_or_error`, `export_of_a_vanished_target_fails_inline` |
| Leaving the form drops an idle flow; a running export keeps it, without the passwords | Pass: `leaving_the_form_drops_an_idle_flow`, `leaving_while_running_keeps_the_flow_without_passwords` |
| Input is ignored while the app is closing or a dialog is open | Pass: `export_input_ignored_while_closing_or_behind_a_dialog` |
| Right-click → Export… on a site and on a workspace opens the screen for that target | Pass: `site_and_workspace_menus` |
| The ⋯ menu and Ctrl+K offer "Export everything…" | Pass: `sidebar_options_offer_export_everything`, `palette_lists_export_everything` |
| The Export… buttons in the site and workspace forms open the screen for the saved entry | Pass: `site_form_export_opens_the_export_screen`, `workspace_form_export_opens_the_export_screen` |
| The three menu items use the shield icon and the Export texts | Pass: `export_labels_and_icon` |
| Every new text in English, pt-BR and Turkish | Pass: `every_en_key_is_used`, `strings_rs_keys_exist`, `catalogs_parse_without_errors`, `same_keys_and_variables_in_both_languages`, `pt_br_values_differ_from_english`, `tr_values_differ_from_english` |
| The screen's look, the dialogs and the file round trips on real machines, in English, pt-BR and Turkish | NEEDS HUMAN: steps below |

### NEEDS HUMAN

Use a vault with at least two sites (one with a saved password) and a workspace with two
sites and saved tabs. Run a debug or release build of this branch: `cargo run -p hoprs-app`.

1. **Look.** Open the export screen for a site, for a workspace and for everything
   (steps 2 and 3 show the ways in). Compare with mockup v2: a form in the middle of the
   window, width about 520 px, neutral near-black, white accent, 1 px rules, 4 px corners, no
   shadow, no modal. The title names the target: `Export "name" (site)`,
   `Export "name" (workspace, 2 sites)`, `Export everything (2 sites, 1 workspace)`. The note
   about partial files shows for a site and a workspace, not for everything. Repeat in
   Settings › Language Português and Türkçe (text fits, nothing cut off). Do it on Linux,
   macOS and Windows.
2. **Right-click.** Right-click a site in the sidebar: "Export…" is in the menu. Choose it:
   the title says `Export "<site name>" (site)`. Do the same on a workspace: the title says
   `(workspace, N sites)`. Close the screen with Cancel and check nothing was written
   (Escape does not close it, as for every form in the middle of the window).
3. **Everything.** Sidebar ⋯ menu → "Export everything…", and Ctrl+K → "Export everything…"
   (Ctrl+Shift+K inside a terminal): both open the screen titled "Export everything". Settings
   → Backup → "Export backup…" opens the same screen; "Restore backup…" is still there.
4. **Keyboard only.** Without the mouse: select a site in the sidebar, press Enter or the
   edit shortcut to open its form, Tab to "Export…" (it shows only when editing, not for a new
   site), press Enter. The screen opens for that site. Tab through File, Choose…, the two
   password fields and Export. Escape does not close the screen (no form in the middle of
   the window closes with Escape): Cancel closes it, or Done after an export. Repeat from a
   workspace's form.
5. **Save dialog and default name.** On the screen click "Choose…": the save dialog proposes
   `hoprs-<site name in lowercase, with - between words>-<today's date>.hoprs` in Documents (or
   home). For everything: `hoprs-backup-<date>.hoprs`. A name with accents or spaces
   (for example "Servidor Principal") gives a slug with `-` and no leading or trailing `-`.
6. **Overwrite.** Type the path of a file that exists in the File field, type the password
   twice and click Export: the screen asks "This file already exists. Overwrite it with the
   new backup?" and writes nothing until you confirm. Cancel leaves the file as it was. A
   path picked in the dialog (which already asked) does not ask again.
7. **Partial file on a second machine.** Export one site (and, again, one workspace) and copy
   the file to a second machine running this build, with a vault that already has other sites.
   Settings → Backup → Restore backup…, choose the file, type the password.
   - Merge: the site (or the workspace and its sites) is added, its password works, the
     sites you had stay.
   - Replace: refused inline with "This file holds only part of a backup. Restore it with
     Merge."; nothing changes (sites, workspaces and passwords are as before; check the
     sidebar and open a site that was there).
8. **Refused by hoprs 1.5.0.** On a machine with hoprs 1.5.0 (the released `.deb`, `.dmg` or
   `.msi`), restore the partial file from step 7 in both modes: the answer is "Unsupported
   backup version" and the vault is untouched (same sites, workspaces and passwords).
9. **Full backup on 1.5.0.** Export everything with this build and restore it with hoprs
   1.5.0, once with Merge and once with Replace: both work as they did for a 1.5.0 backup.

## Google Drive destination (2026-11-23)

Behaviour as of pull request #48 (`drive-enable`). The Pass results come from that pull request's CI (ubuntu, macos, windows), not from runs on this branch. Nothing here needs a Google account except
the rows under "End-to-end run" and "NEEDS HUMAN (Google Drive)".

| Check | Result |
| --- | --- |
| The OAuth client needs both values, an empty value counts as unset, run-time values count only in a debug build; the only scope is `drive.file`; the endpoints are Google's or those under `HOPRS_DRIVE_URL` (`https://` only) | Pass: `client_needs_both_values`, `run_time_values_only_in_debug`, `only_the_drive_file_scope`, `endpoints_under_a_base` |
| PKCE (RFC 7636), a different `state` and verifier each time, the authorisation URL has exactly its parameters, the callback parser, no code in `Debug` | Pass: `pkce_matches_rfc_7636`, `state_and_verifier_differ_each_time`, `auth_url_has_exactly_the_parameters`, `callback_parser`, `callback_debug_hides_the_code` |
| The loopback listener binds `127.0.0.1` only; a right `state` with a code returns it, with an error is "denied"; a stray request, a wrong `state`, junk, a silent connection or a connection that falls over do not end the wait; a request head far over 8 KiB (large cookies) still connects; a long request line is refused; Cancel and the time-out free the port | Pass: `good_callback_returns_the_code_and_the_page`, `denied_callback_fails_with_the_failed_page`, `stray_requests_do_not_end_the_wait`, `connections_that_fall_over_do_not_end_the_wait`, `a_big_request_head_does_not_matter`, `a_long_request_line_is_refused`, `cancel_and_timeout_free_the_port`, `only_a_broken_listener_ends_the_wait`, `the_returned_code_is_not_in_debug_output`, `pages_escape_their_text` |
| The client: the PKCE form, refresh and revoke, the `hoprs` folder found or created with an exact query, the multipart upload, an unsafe view link left out, unexpected answers are errors, no redirect is followed, a token goes only to its own endpoint, a file over 5 MB is refused without a request | Pass: `exchange_sends_the_pkce_form`, `refresh_and_revoke`, `folder_is_found_or_created`, `folder_query_is_exact`, `upload_sends_metadata_and_bytes`, `unsafe_view_link_is_left_out`, `unexpected_answers_are_errors_not_panics`, `redirects_are_refused`, `tokens_go_only_to_their_endpoint`, `upload_over_five_megabytes_is_refused_without_a_request` |
| The connection: one keyring entry holds the refresh token and the email; with the keyring down the connection lasts for the session; an unreadable keyring is an error and is asked again; status reads the keyring only when asked; a missing or garbage entry is "not connected"; an upload refreshes, finds the folder, uploads; a refused refresh drops the connection; Disconnect revokes and deletes | Pass: `connect_stores_token_and_email_in_one_entry`, `keyring_down_keeps_the_connection_for_the_session`, `unreadable_keyring_is_an_error_and_is_asked_again`, `status_reads_the_keyring_only_when_asked`, `missing_or_garbage_entry_is_not_connected`, `upload_refreshes_then_finds_the_folder_and_uploads`, `refused_refresh_drops_the_connection`, `disconnect_revokes_and_deletes` |
| Cancel ends the wait of its own attempt only, early or late | Pass: `cancel_ends_the_wait`, `ids_decide_which_attempt_is_current`, `cancel_touches_only_its_own_attempt`, `a_cancel_before_the_attempt_registered_ends_it`, `a_late_earlier_attempt_does_not_cancel_the_current_one` |
| No token, code, verifier or client secret in `Debug` or in an error | Pass: `nothing_secret_in_debug_or_errors` |
| A build without a client: every call answers `drive-not-configured`; the export screen offers no Destination | Pass: `not_configured_answers_every_call`, `drive_calls_without_a_client`, `drive_is_not_offered_without_a_client` |
| The keyring entry is service `hoprs/google-drive`, account `refresh-token` | Pass: `app_secret_names` |
| `settings.toml` holds `google_drive = true` only when set, ignores a value that is not a boolean, keeps unknown keys | Pass: `google_drive_marker_roundtrip_and_invalid` |
| The flow: connect waits then connects; a second Connect click is ignored; export seals then uploads; a failed upload offers Try again and Save to this computer; a refused authorisation asks to connect again and clears the marker; one export at a time; Disconnect clears the marker; a stale marker is cleared when the entry is gone | Pass: `connect_waits_then_connects`, `second_connect_click_is_ignored`, `export_to_drive_seals_then_uploads`, `upload_failure_offers_try_again_and_save`, `refused_authorisation_asks_to_connect_again`, `one_export_at_a_time_for_drive_too`, `disconnect_clears_the_marker`, `stale_marker_is_cleared_when_the_entry_is_gone` |
| Opening the form never reads the keyring; an unreadable keyring is asked again on the next pick | Pass: `opening_the_form_never_reads_the_keyring`, `unreadable_keyring_is_asked_again` |
| A connection that is stored sets the marker; one held for this session only sets none and leaves an existing one | Pass: `a_session_only_connection_sets_no_marker`, `a_session_only_connect_keeps_an_existing_marker` |
| The name field must not be empty (`export_to_drive_seals_then_uploads`); the form's messages and effects reach the backend | Pass: `drive_messages_from_the_user`, `drive_effects_reach_the_backend`, `submit_does_nothing_after_an_upload_or_a_local_save`, `a_failed_upload_can_be_submitted_again`, `drive_error_goes_when_the_form_is_left` |
| The form's Drive row and outcome per state; nothing of Drive under the File destination | Pass: `drive_row_per_link`, `drive_outcome_per_state`, `no_drive_row_for_the_file_destination`, `the_file_destination_shows_nothing_of_a_drive_outcome` |
| Settings → Backup shows the line from the marker or a live connection, only in a build with Drive; a failure shows without the marker | Pass: `drive_line_follows_the_marker_and_the_build`, `drive_line_also_follows_a_live_connection`, `drive_error_shows_without_the_marker_but_only_in_a_drive_build` |
| Every new text in English, pt-BR and Turkish | Pass: `every_en_key_is_used`, `same_keys_and_variables_in_both_languages`, `error_codes_exist_in_both_catalogs`, `pt_br_values_differ_from_english`, `tr_values_differ_from_english` |

## Google Drive: end-to-end run (2026-11-23, not in CI)

What ran: hoprs's own `drive` module, driven from a test harness with an in-memory secret
store, on Linux, against the real Google project (app in production, brand verified, a Desktop
app OAuth client, the single scope `drive.file`).

What it did **not** go through: the app's window, a release build, the system keyring, macOS
or Windows. Every row below that depends on one of these stays NEEDS HUMAN.

Result:

- The consent screen showed the app name `hoprs` and the permission "See, edit, create, and
  delete only the specific Google Drive files you use with this app". It showed **no**
  unverified-app warning. It also showed an informational box about missing Privacy Policy or
  Terms of Service links; hoprs has no terms page.
- The token exchange returned a refresh token. The account's email was readable (`about.get`
  answers under `drive.file`), so the form can show it.
- Three uploads went into the `hoprs` folder, one of them after a token refresh.
- Disconnect revoked the access at Google.

## NEEDS HUMAN (Google Drive)

Use a release build or a build with `HOPRS_GOOGLE_CLIENT_ID` and `HOPRS_GOOGLE_CLIENT_SECRET`
set, and a real Google account. Do each row on Linux, macOS and Windows unless it says
otherwise. Use a vault with a site that has a saved password.

1. **Connect.** Right-click a site, Export…, Destination "Google Drive", "Connect to Google
   Drive…". The browser opens on Google's consent screen: app name `hoprs`, only "See, edit,
   create, and delete only the specific Google Drive files you use with this app". (The run
   above saw no unverified-app screen, only an informational box about missing Privacy Policy
   or Terms links: note whether that is still so, and with which account type.) After allowing,
   the browser tab says "Google sent the answer to hoprs. Go back to hoprs to see the result;
   you can close this tab." and hoprs shows the account (its email, or "Connected": record which). Do it
   with Português and Türkçe too (the tab's page is in the app's language).
2. **Export and find the file.** Type the password twice and Export. The screen says
   "Uploaded to Google Drive: …". In Drive there is a folder `hoprs` holding the file. "Open in
   Drive" opens it in the browser.
3. **Restore.** Download the file from Drive, and on a machine with other sites restore it with
   Settings → Backup → Restore backup… and Merge: the site is added and its password works.
   Replace is refused for a partial file.
4. **A second export** the same day, with the same name, gives a second file (nothing is
   overwritten).
5. **Cancel and no browser.** Click Connect, then Cancel: the wait ends. Click Connect again
   with no browser: on Linux start hoprs with a no-op `xdg-open` first on `PATH`, or just close the tab that opened. Then use "Copy the link" and paste it in another browser. Any browser on this computer completes the wait, since only the `state` matters. Leave one connect unanswered for 5
   minutes: "The browser did not answer in 5 minutes" (Linux is enough).
6. **Offline.** Disconnect the network, Export: an error shows inline and the screen offers
   "Try again" and "Save to this computer…". Turn the network back on, then "Try again" uploads; or save to this
   computer and restore the saved file.
7. **A file over 5 MB** (a throwaway config dir, for example `XDG_CONFIG_HOME=<dir>` on Linux, with a generated `sites.toml` of tens of thousands of sites copied from a real entry with distinct ids; export everything. First export to a file and check that it is over 5 MB; if not, add more sites. Key files are never in an export, and the limit has no debug override): the upload fails with "Could not upload to Google Drive" and "Save to
   this computer…" works (Linux is enough).
8. **Disconnect** in the export screen, and again in Settings → Backup (connect first): the
   keyring entry goes, `settings.toml` loses `google_drive`, and the app disappears from
   <https://myaccount.google.com/permissions>.
9. **Revoked at Google.** Connect, remove hoprs at
   <https://myaccount.google.com/permissions>, Export: the error "Not connected to Google
   Drive" shows, Connect is shown again, `google_drive` leaves `settings.toml`, and "Save to this computer…" still saves the file.
10. **The keyring.** After connecting, the keyring has one entry, service `hoprs/google-drive`,
    account `refresh-token` (Seahorse; Keychain Access; Credential Manager shows
    `refresh-token.hoprs/google-drive`), and `settings.toml` holds `google_drive = true` and
    nothing else about Google. Restart hoprs: Settings → Backup shows "Google Drive:
    connected"; pick Google Drive in an export: it shows the account and uploads without a new
    sign-in. With the keyring locked or absent (Linux: lock the login keyring and refuse the unlock prompt, or start hoprs with `env -u DBUS_SESSION_BUS_ADDRESS`): the screen
    says "The system keyring is unavailable: this connection lasts until hoprs closes.", no
    marker is written, and after a restart Connect is shown again.
11. **A browser with large cookies for `127.0.0.1`** (visit a local dev server that sets big
    cookies first) still connects (Linux is enough).
12. **A build without the client** (a source build, or the CI artifact of a pull request)
    shows no Destination row on the export screen and no Google line in Settings.
13. **The log panel** shows lines such as `Google Drive: connected` and `export uploaded to Google Drive name=<name>`, and no token, code or secret.
14. **The window.** The run above never went through the app's window. Look at the Drive
    rows at 520 px (Account with Disconnect, Name, the "Waiting for the browser…" state with
    Cancel and "Copy the link"), that Export is disabled until connected, and that nothing
    shows a Google or Drive logo.
