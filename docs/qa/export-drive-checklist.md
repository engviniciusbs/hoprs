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
| Every new text in English, pt-BR and Turkish | Pass: `every_en_key_is_used` and the catalog parity tests |
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
   `(workspace, N sites)`. Close the screen (Escape or the sidebar) and check nothing was
   written.
3. **Everything.** Sidebar ⋯ menu → "Export everything…", and Ctrl+K → "Export everything…"
   (Ctrl+Shift+K inside a terminal): both open the screen titled "Export everything". Settings
   → Backup → "Export backup…" opens the same screen; "Restore backup…" is still there.
4. **Keyboard only.** Without the mouse: select a site in the sidebar, press Enter or the
   edit shortcut to open its form, Tab to "Export…" (it shows only when editing, not for a new
   site), press Enter. The screen opens for that site. Tab through File, Choose…, the two
   password fields and Export; Escape leaves. Repeat from a workspace's form.
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
