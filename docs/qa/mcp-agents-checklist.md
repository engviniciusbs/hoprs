# MCP server for agents checklist (Plan 17, 1.6.0)

Automated rows first, then the steps that need a person, a real MCP client and, for most of
them, three operating systems. The wire tests drive the relay over in-memory streams in both
protocol eras (2025-11-25 and 2026-07-28); no real MCP client had run against it when these
rows were written, so the NEEDS HUMAN rows are the first real run.

## The permission, the tools and the channel (2026-11-22)

| Check | Result |
| --- | --- |
| The level (`No access`, `Allow reading`, `Allow everything`) is stored in `sites.toml` (absent means none, an unknown value reads as none and the site stays readable) | Pass: `agent_access_serde`, `agent_access_order`, `agent_access_round_trips_in_sites_toml`, `unknown_agent_access_is_no_access_not_unreadable` |
| A backup, a partial export and the FileZilla import never carry or set the level | Pass: `export_never_carries_the_access_level`, `restore_never_sets_the_access_level`, `imported_sites_have_no_agent_access` |
| The tool table: thirteen tools in order, unique valid names, closed object schemas, argument types and bounds, titles, the hints (read-only, destructive, idempotent, open-world) and each tool's level | Pass: `thirteen_tools_in_order`, `names_are_valid_and_unique`, `every_schema_is_a_closed_object`, `argument_types_and_bounds`, `titles_match_the_table`, `hints_follow_the_table`, `levels_are_pinned` |
| The descriptions name their caps, levels and what is destructive | Pass: `descriptions_name_their_caps`, `descriptions_state_levels_and_destructiveness` |
| The access matrix: every tool at every level on SSH and on FTP; a site with no access and a site that does not exist read the same; a level change applies on the next call; a deleted site reads as unavailable; admission follows what the tool needs, not its arguments | Pass: `access_matrix`, `no_access_and_no_such_site_read_the_same`, `level_change_applies_on_the_next_call`, `deleted_site_reads_as_unavailable`, `admission_follows_the_need_not_the_arguments`, `upload_needs_everything_download_needs_reading` |
| `run_command` on an FTP site is refused and the host is never called | Pass: `run_command_reply_and_limits`, `access_matrix` |
| The access changes while an agent works: the call running finishes, an agent-only session of a site that lost access is closed, the switch turned off closes agent sessions at once and refuses new calls | Pass: `session_of_a_site_that_lost_access_is_closed`, `switch_off_closes_agent_sessions_at_once`, `switch_off_refuses_everything`, `switch_off_lets_the_running_call_finish` |
| A wait that can last (the site's command turn, a connection attempt) is followed by a new admission: a level lowered to No access or Allow reading, or the switch turned off, meanwhile refuses the call with the text and log line of a call refused at once, and nothing is done | Pass: `a_call_is_admitted_again_after_its_connect`, `a_command_is_admitted_again_after_its_turn` |
| A site edited to another server (protocol, host, port or user) while a session is open: an agent-only session is closed and the call connects anew; a tab's session is neither closed nor used and the call says the user must reconnect the site; a session an unfinished transfer uses is left alone; a site edited during the connect is connected again; an unchanged server keeps its session; the sessions record where they connect | Pass: `an_edited_site_gets_a_new_agent_session`, `an_edited_site_is_refused_on_a_tabs_session`, `an_edited_site_leaves_a_running_transfer_alone`, `a_site_edited_during_the_connect_is_connected_again`, `an_unchanged_server_keeps_its_session`, `connect_for_agent_starts_an_agent_session`, `host_sessions_are_the_agents` |
| `list_sites` carries no key file, no path and no secret flag | Pass: `list_sites_has_no_key_file_and_no_secret_flags` |
| A connection failure reaches the agent as a fixed sentence, never the error's own text, `detail` or a key path | Pass: `connect_errors_are_fixed_sentences` |
| The log holds one line per call, never file content, command output or a secret; control characters cannot split or forge a line; one line is written for a cancelled call and for a long command | Pass: `one_log_line_per_call`, `log_has_no_content_and_no_secret`, `log_lines_cannot_be_split_or_reordered`, `a_cancelled_call_is_logged`, `a_long_command_keeps_its_outcome_in_the_log`, `write_tools_log_what_they_did`, `transfer_calls_are_logged` |
| No secret reaches a log line or a `Debug` print: connect secrets, prompt answers, typed passwords, message content, protected roots, command output | Pass: `connect_secrets_never_reach_the_log`, `prompt_answers_never_reach_the_log`, `failed_upload_never_logs_the_content`, `debug_never_prints_content`, `the_roots_never_show_in_debug`, `debug_shows_lengths_and_never_the_output` |
| `rmcp`'s debug log (which can print an input line) is capped whatever `RUST_LOG` says | Pass: `real_filter_never_logs_rmcp_debug` |
| Listings are capped and paged with a total; `stat` and listing shapes; `read_file` ranges, binary and oversized ranges, zero-size files, a link as the last component; numbers at `u64::MAX` do not panic | Pass: `huge_listing_is_capped_and_paged`, `listing_and_stat_shapes`, `read_file_ranges`, `read_file_refuses_binary_and_oversized_ranges`, `zero_size_files_are_read_by_content`, `links_are_refused_up_front`, `numbers_at_u64_max_do_not_panic`, `odd_arguments_are_tool_errors` |
| `write_file` rules (overwrite false, folders, links, 1 MiB); make, rename, change permissions; delete needs `recursive` for a full folder and never walks without it; `..` in a path is refused | Pass: `write_file_rules`, `make_rename_chmod`, `delete_needs_recursive_for_a_full_folder`, `delete_without_recursive_never_walks`, `dot_dot_paths_are_refused` |
| `write_file` goes through the remote-edit writer: temporary file and rename, or in place; a failed or cut write cleans up and says what is left | Pass: `put_writes_by_mode`, `put_failures_clean_up_and_say_what_is_left` |
| `run_command`: reply shape, limits, one at a time per site, a waiting command leaves the queue when cancelled, a network error says the connection may be lost | Pass: `run_command_reply_and_limits`, `commands_run_one_at_a_time_per_site`, `a_cancelled_waiting_command_leaves_the_queue`, `a_network_exec_error_says_the_connection_may_be_lost` |
| The exec channel: the two streams, the status or signal, output capped per stream while draining, the time limit, a status seen at the deadline, a lost channel, `Debug` showing lengths only | Pass: `collect_splits_the_streams_and_reads_the_status`, `collect_caps_each_stream_and_keeps_draining`, `collect_reports_a_signal_and_a_lost_channel`, `the_deadline_ends_a_flood_and_a_silent_command`, `a_status_seen_before_the_deadline_wins_over_the_time_out`, `a_status_queued_at_the_deadline_is_not_lost`, `no_status_at_the_deadline_is_a_time_out`, `other_extended_streams_are_ignored` |
| Exec against the real OpenSSH server in Docker (the ignored tests, run with Docker up): output and exit code, no stdin and no terminal, the time limit closes the channel and kills the process, large output capped, the channel budget, dropping frees the channel | Pass: `exec_returns_output_and_exit_code`, `exec_has_no_stdin_and_no_terminal`, `exec_time_limit_closes_the_channel`, `exec_large_output_is_capped`, `exec_counts_against_the_channel_budget`, `exec_when_not_connected_fails_at_once`, `dropping_exec_frees_the_channel`, `exec_time_limit_kills_the_remote_process`, `dropping_exec_kills_the_remote_process` |
| `download` and `upload` enqueue with the policy and the origin `Agent`; they answer the summary, "skipped" for an existing file, "running" after the wait; `transfer_status` is for agent transfers only; an agent transfer never waits on a question | Pass: `download_and_upload_enqueue_with_their_policy`, `finished_transfer_answers_its_summary`, `skipped_single_file_says_skipped`, `slow_transfer_answers_running`, `transfer_status_is_for_agent_transfers_only`, `agent_never_waits_on_a_question`, `the_service_transfers_through_the_engine` |
| A folder's children inherit the policy and the origin; both survive a restart; an agent enqueue needs a policy that never asks; the guard runs for every agent item | Pass: `folder_policy_reaches_every_descendant`, `folder_skip_policy_skips_what_exists`, `origin_reaches_every_descendant`, `inherited_policy_and_origin_survive_a_restart`, `agent_enqueue_needs_a_policy_that_never_asks`, `agent_guard_runs_for_every_agent_item`, `tree_policy_and_origin_round_trip` |
| The queue schema is v4: a v3 file opens as v4, a newer schema is refused (so hoprs 1.5.0 refuses this version's queue) | Pass: `v3_queue_opens_as_v4`, `a_newer_schema_is_refused` |
| Protected local paths: each root refused as itself, inside it and around it; neighbours stay open; links, dangling links, hard links and aliases of a root; letter case and Windows spellings; an ancestor refuses only a folder transfer; every hoprs directory and `~/.ssh` and `known_hosts` are roots | Pass: `each_root_is_refused_as_itself_inside_and_around`, `neighbours_stay_open`, `links_are_resolved`, `a_dangling_link_is_followed_to_where_it_would_write`, `a_hard_link_to_a_key_file_is_the_key_file`, `an_alias_of_a_protected_folder_is_that_folder`, `letter_case`, `windows_spellings_of_one_path`, `an_ancestor_of_a_root_refuses_only_a_folder_transfer`, `dots_and_separators_do_not_walk_around_a_root`, `fixed_roots_cover_every_hoprs_directory`, `prepared_checks_agree_with_check` |
| Key files named by saved sites are roots, whatever their agent level; a refused transfer names only the path the agent gave; a protected path is checked before connecting when it can be | Pass: `protected_names_hoprs_dirs_and_key_files`, `protected_paths_stop_the_transfer`, `protected_is_checked_before_connecting_when_it_can_be`, `agent_transfers_are_guarded` |
| The running program is protected: the executable (through a link too) and, on macOS, its `.app` bundle; a file beside it stays open; the backend's guard refuses it both ways | Pass: `the_program_is_protected`, `a_program_inside_an_app_bundle_protects_the_bundle`, `protected_names_hoprs_dirs_and_key_files`, `agent_transfers_are_guarded` |
| A symbolic link inside an uploaded folder is skipped, never opened | Pass: `symlink_is_skipped`, `root_symlink_is_skipped` |
| Local path rules: absolute, no `..`, one Windows drive, a regular file or a folder only | Pass: `local_path_rules`, `engine_path_rules`, `engine_path_rules_unix`, `engine_path_rules_windows` |
| The channel on Unix: directory `0700`, socket `0600`, the peer's user id checked, a symlinked directory and another user's directory refused, a stale socket replaced, a file that is not a socket never replaced, the socket removed on drop | Pass: `socket_and_directory_modes`, `directory_rule`, `peer_rule`, `same_user_connects`, `a_symlinked_directory_is_refused`, `a_directory_of_another_user_is_refused`, `stale_socket_is_replaced`, `what_is_not_a_socket_is_never_replaced`, `drop_removes_the_socket`, `drop_leaves_a_newer_socket_alone` |
| The channel on Windows: the pipe's name carries the SID, its security descriptor admits only the user, a second first instance is refused | Pass (Windows CI job): `pipe_name_carries_the_sid`, `pipe_admits_its_owner`, `second_first_instance_is_refused`, `descriptor_names_only_the_user` |
| The channel's messages: the wire shapes, the hello and its limits, malformed lines are errors and never a panic, lines cut at the limit | Pass: `wire_shapes`, `hello_rule`, `the_hello_is_small`, `malformed_messages_are_errors_not_panics`, `read_line_limits`, `read_line_does_not_buffer_past_the_limit`, `read_message_wants_one_json_object` |
| A hostile peer: junk and oversized lines close the connection, a silent client is dropped, a mismatch is answered and closed, duplicate ids close it, a relay that floods without reading is closed, a ninth call waits, a seventeenth relay is refused | Pass: `junk_and_oversized_lines_close_the_connection`, `silent_client_is_dropped`, `mismatch_is_answered_and_closed`, `duplicate_ids_close_and_unknown_cancels_are_ignored`, `a_relay_that_floods_without_reading_is_closed`, `a_relay_that_does_not_read_holds_up_nobody`, `a_ninth_call_waits_its_turn`, `a_seventeenth_call_is_refused_and_cancels_are_still_read`, `seventeenth_relay_is_refused`, `hellos_in_progress_are_capped`, `a_call_whose_task_panics_is_answered` |
| Calls overlap and match their ids; a cancel stops a call and sends no answer; calls end with their relay | Pass: `hello_then_calls`, `calls_overlap_and_match_their_ids`, `cancel_stops_a_call_and_sends_nothing`, `calls_end_with_their_relay` |
| The switch closes the channel, drops the relays and removes the socket; it follows the latest value; bind failures are reported and retried; no endpoint means no listening; the app quitting says `quit` | Pass: `switch_off_drops_relays_and_removes_the_socket`, `switch_follows_its_latest_value`, `bind_failure_is_reported_and_retried`, `no_endpoint_never_listens`, `shutdown_says_quit`, `agents_switch_starts_from_the_settings_file`, `agent_channel_follows_the_settings`, `host_follows_the_switch` |
| The reaper closes agent-only sessions after ten idle minutes, leaves sites with a call running, and runs with the switch off | Pass: `idle_sessions_close_after_ten_minutes`, `reap_leaves_sites_with_calls_in_flight`, `reaper_runs_with_the_switch_off` |
| The relay speaks both protocol eras: the legacy handshake, modern requests with no handshake, the served list is the table, tool entries carry hints and schemas | Pass: `legacy_handshake_then_tools`, `modern_requests_without_a_handshake`, `served_list_is_the_table`, `tool_entries_carry_hints_and_schemas` |
| The relay's errors: an unknown tool is a protocol error; bad or missing arguments are tool errors; hoprs closed or the switch off, a version mismatch and `bye` each have their own text; the relay reconnects by itself when the app returns, and an app that never greets is unreachable | Pass: `unknown_tool_is_a_protocol_error`, `bad_arguments_are_a_tool_error`, `missing_arguments_reach_the_app_as_an_object`, `unreachable_app_is_a_tool_error`, `mismatch_and_bye_have_their_own_texts`, `reconnects_when_the_app_returns`, `an_app_that_never_greets_is_unreachable`, `a_call_goes_to_the_app_and_back`, `calls_overlap`, `cancel_reaches_the_app` |
| A version mismatch names the older side's remedy: restart hoprs for an older window, restart or reconnect the MCP server for an older relay, both when the versions do not compare; the relay's log has no colour codes off a terminal | Pass: `version_mismatch_names_the_older_side`, `relay_log_has_no_colour_codes_off_a_terminal` |
| Nothing but protocol on stdout: the relay's output is JSON-RPC lines, no source under `agent/` prints, and the real binary's stdout is protocol only | Pass: `output_is_json_rpc_lines_only`, `agent_sources_never_print`, `stdout_is_protocol_only` |
| The relay ends when its input ends; `hoprs mcp` takes no instance lock; the usage names `mcp` | Pass: `ends_with_its_input`, `ends_with_its_input_before_any_opening`, `mcp_does_not_take_the_instance_lock`, `help_names_mcp`, `parse_args_mcp` |
| The relay reaches a real server and fails cleanly without a channel | Pass: `dial_reaches_the_real_server`, `dial_without_a_channel_fails` |
| A session an agent starts is marked agent-only until a tab takes it over; closing it spares a tab's session; a prompt it raises is flagged | Pass: `connect_for_agent_starts_an_agent_session`, `agent_session_is_marked_and_a_tab_takes_it_over`, `close_agent_session_spares_a_tab_session`, `connect_for_agent_retries_only_its_own_session`, `agent_prompts_are_flagged`, `host_sessions_are_the_agents`, `connects_once_and_reuses`, `failed_agent_connect_closes_its_session` |
| A prompt the user does not answer ends in "waiting for the user" after the wait; an abandoned attempt stays out of the user's way; waiters keep their answer | Pass: `prompt_wait_ends_with_the_right_text`, `an_abandoned_attempt_is_out_of_the_users_way`, `a_cancelled_waiter_leaves_the_others_their_answer`, `concurrent_calls_share_one_connection` |
| The status-bar indicator follows the relays; agent events never touch tabs or focus; the queue tag; the sidebar mark; the agent line in the security dialogs; a dialog shows without a tab and survives an abandoned site | Pass: `agent_indicator_follows_the_relays`, `agent_events_never_touch_tabs_or_focus`, `agent_events_land_while_closing`, `agent_rows_have_the_tag`, `site_state_for_an_agent_session`, `agent_dialogs_say_so`, `agent_prompt_is_shown_without_a_tab`, `an_agent_prompt_survives_an_abandoned_site`, `saved_secret_from_an_agent_dialog_reloads_profiles`, `agent_feed_delivers_events` |
| Enter and Escape on an agent's host-key dialog answer Cancel | Pass: `agent_host_key_dialog_keys_cancel` |
| The site form's level section starts from the site, is saved, shows one hint per level and protocol, and keeps the level when the protocol changes | Pass: `agent_access_starts_from_the_site_and_is_saved`, `agent_hint_per_level_and_protocol`, `changing_the_protocol_keeps_the_level` |
| Settings › Agents (MCP): the switch saves and reaches the backend; the command quotes any path; the JSON is valid; the level counts; a channel error shows | Pass: `agents_switch_saves_and_reaches_the_backend`, `agent_command_quotes_the_path`, `agent_json_is_valid_json`, `agent_levels_counts_each_level`, `agents_section_shows_the_channel_error`, `every_section_has_title_description_and_icon` |
| Every new text in English, pt-BR and Turkish | Pass: `every_en_key_is_used`, `same_keys_and_variables_in_both_languages`, `error_codes_exist_in_both_catalogs`, `pt_br_values_differ_from_english`, `tr_values_differ_from_english` |
| The look and behaviour on real machines, with a real MCP client, in English, pt-BR and Turkish | NEEDS HUMAN: steps below |

## End-to-end run (2026-11-22, not in CI)

What ran: the real app, the real `hoprs mcp` relay and the Docker sshd together on Linux, in
a container with a virtual display (a debug build, a scripted MCP client speaking both
protocol eras).

Result: all twelve steps passed. The thirteen tools; the three levels; a "No access" site
answering exactly like an unknown one; nine protected-path refusals, the program itself
included; a timed-out command (killed on the server); the switch turned off in the UI during
a running call; hoprs not running; an unknown host key raised by an agent (the dialog with
its "An agent asked for this connection." line, the two-minute wait, the answer while a call
waits). The indicator, the queue tag and the sidebar mark were seen in screenshots.

What it does not replace: a real Claude Code session, macOS and Windows, a normal window
close, FTP sites, a changed host key, password and passphrase prompts, a version mismatch,
the limits on relays and calls, the idle close. The NEEDS HUMAN rows below keep their
status; rows 1 and 2 stay required before the 1.6.0 tag.

## NEEDS HUMAN

Run each row on Linux, macOS and Windows unless it says otherwise. Use a vault with four
sites: an SSH site set to **Allow everything** (call it `rw`), an SSH site set to **Allow
reading** (`ro`), an FTP site set to **Allow everything** (`ftp`) and one SSH site left at
**No access** (`none`). Install a release build (`.deb`, `.dmg`, `.msi`) or run
`cargo run -p hoprs-app`. Use a test server for `rw`: the agent will write and delete there.
Have Claude Code (a recent version) in a terminal on the same computer.

1. **Register.** In hoprs, Settings › Agents (MCP): the switch is off. Turn it on. The
   section shows the command and a JSON snippet, each with Copy, and a line with how many
   sites have each level. Copy the command and run it in a terminal. On Linux it names
   `/usr/bin/hoprs` (the `.deb`); on macOS the path inside `hoprs.app`
   (`/Applications/hoprs.app/Contents/MacOS/hoprs`); on Windows
   `C:\Program Files\hoprs\hoprs.exe`. Start `claude`, run `/mcp`: hoprs is listed, connected,
   with thirteen tools. The status bar shows "Agent connected" once Claude Code connected.
2. **Each tool once** (ask Claude Code in plain words, then check hoprs's log: one `agent`
   line each, with the site's name):
   1. *List sites:* `rw`, `ro` and `ftp` are listed with their level; `none` is not there.
      Ask for `none` by name: the agent cannot find it, with the same answer it gets for a
      site that does not exist.
   2. *List a folder* on `ro`; ask for the second page of a big folder (`offset`).
   3. *Describe a path (`stat`)* on `ro`: once for a file and once for a folder; the answer
      names the kind, size, modification time and permissions.
   3b. *Read a file* on `ro`: its text comes back; a binary file is refused with a pointer to
      download.
   4. *Download* a file and then a folder from `ro` into a new folder of your home. Rows
      with the "agent" tag appear in the queue and finish. Download again: the files are
      skipped (overwrite is false); ask for overwrite: they are replaced. For a slow
      download (a large file, or throttle the network) the answer says "running" with a
      transfer number: ask for `transfer_status` on that number, then again after it
      finishes: "running", then "done" with the counts.
   5. *Write a file* on `rw`, then edit it (read, then write with overwrite). The same on
      `ro`: refused with a message that says the site allows reading only.
   6. *Make a folder* on `rw`; *rename* the file into it; *change its permissions* to `600`
      (on `ftp` the server may refuse: the agent gets an ordinary error).
   7. *Upload* a file and a folder from your computer to `rw`: tagged rows in the queue.
   8. *Delete* the uploaded file, then the folder (it needs `recursive`).
   9. *Run a command* on `rw` (`uname -a`, then `sleep 120` with `timeout_seconds` 5). The
      first answers with its output and exit code. See step 11 for the second.
   10. Ask for a command on `ftp`: refused with "The site \"<name>\" is not an SSH site, and only SSH sites can run commands. Use the file tools on it instead."
3. **No tab, no focus, no sound.** While the agent works, no tab opens, the focus stays where
   it was (type in a rename field in a pane while the agent lists folders), and nothing
   makes a sound. The sidebar shows the site as "Connected (by an agent)" (hover or the
   tooltip), and when a tab opens the same site it takes the session over, with its shell
   and edits untouched.
4. **A host-key question.** Remove the site's line from `known_hosts` (or use a new site),
   then ask the agent to list a folder. The security dialog appears **in the hoprs window**
   with the line "An agent asked for this connection." Enter still means Cancel. Answer
   Trust: the agent's call then succeeds. Repeat and press Cancel: the agent gets an error.
   A password prompt (a site with no saved password) behaves the same way, and a typed
   password with "save" ticked is stored once the connection works.
5. **A question left unanswered.** Raise the dialog again and leave it for two minutes: the
   agent gets "hoprs asked the user a question about this connection… and got no answer
   within two minutes…", the dialog disappears, and the session is closed. Ask again: the
   dialog comes back and waits two minutes.
6. **A changed host key.** Edit `known_hosts` so the key of `rw` is wrong: the agent gets
   "The server's host key changed…" and the hoprs window offers no accept for it.
7. **The switch turned off mid-session.** Turn Settings › Agents (MCP) off while the agent is
   connected: "Agent connected" goes away, the agent's sessions close (the sidebar loses
   the "by an agent" mark), the next call fails with "Agent access was turned off…", and the
   socket (Unix: `agent.sock` beside `instance.lock`) is gone. Turn it on: the next call
   works without restarting Claude Code.
8. **A site with no access is invisible.** Set `rw` to No access in its form: the next call
   on it answers exactly as for an unknown id, `list_sites` omits it, and a call already
   running was not cut. Set `ro` to Allow everything and back: the change applies on the
   next call. The form shows one hint per level, and none for No access.
9. **Restore and import never give access.** Export a site that is on Allow everything,
   delete it, restore it (Merge): it comes back at No access. Import a FileZilla file:
   the new sites are at No access.
10. **hoprs closed and reopened.** Quit hoprs while Claude Code is open: the next call says
    hoprs is not reachable, and says so again for as long as hoprs is closed. Open hoprs
    (the switch stays on): the next call works with no restart. Also start Claude Code
    first, with hoprs closed, then open hoprs: the first call after works.
11. **A timed-out command.** Ask for `sleep 120` with `timeout_seconds` 5 on `rw`: the answer
    says it timed out after about five seconds. On the server, check `ps`: the process is
    gone (the KILL arrived) or, if it is still there, the answer said it might be. Ask for
    a command that never ends and cancel it in Claude Code (Esc): the process on the server
    ends too, or, since the KILL is best effort, it may keep running (check `ps`); the log line says cancelled.
12. **An updated binary against the old window.** With hoprs running, replace the
    executable on disk (install a newer build over it, or copy one in) without quitting
    hoprs, then start a new agent session (a new relay) and make a call: the agent gets
    "hoprs was updated while its window stayed open (window <old>, this program <new>).
    Restart hoprs, then call the tool again." Restart hoprs: it works again.
13. **A security prompt cannot be answered by an agent.** With a dialog open (step 4),
    ask the agent to call another tool on the same site: it waits or gets the "waiting for
    the user" text, and nothing it sends answers the dialog.
14. **Protected local paths.** Ask the agent, on `ro`, to download a remote file to
    `~/.ssh/known_hosts`, to `~/.ssh/anything`, to hoprs's `sites.toml` (Linux
    `~/.config/hoprs/sites.toml`; macOS `~/Library/Application Support/hoprs/sites.toml`;
    Windows `%APPDATA%\hoprs\sites.toml`), to `settings.toml` and to a key file a saved site
    names (a site with "Key file" set; keep a copy of the key). Each is refused with "…is
    protected…" and nothing is written. Then on `rw` ask for an upload of the key file, of
    `known_hosts`, and of a **folder that contains** a key file: all refused (the folder's
    refusal names the folder the agent gave, not the key). A file next to them still works.
    With the transfer queue open, no row appears for a refused path.
15. **macOS: the second path of the home directory.** On macOS only: ask for a download to
    `/System/Volumes/Data/Users/<you>/.ssh/known_hosts` and for an upload of
    `/System/Volumes/Data/Users/<you>/.ssh/<a key>`: both are refused. This is reasoned in
    the code (a file identity comparison) and has never run on a Mac; if either succeeds,
    stop and report it.
16. **Symbolic links.** On Linux and macOS, create a link in your home to `~/.ssh` and ask for
    a download into it: refused. Put a link to a key file inside a folder, upload the folder:
    the link is skipped and nothing under `~/.ssh` is read.
17. **`read_file` on a link.** Ask the agent to read a remote symbolic link: the answer
    refuses and points to `download`.
18. **Another user cannot connect.** Linux and macOS: with a second account, run
    `sudo -u other nc -U <path>/agent.sock` (the directory of `instance.lock`): refused or
    closed with no answer, and the hoprs log shows nothing from it. Windows: a second account
    cannot open `\\.\pipe\hoprs-agent-<SID>` (for example from PowerShell with
    `[IO.Pipes.NamedPipeClientStream]`: access denied). Nothing listens on the network:
    `ss -ltnp | grep hoprs` (Linux) shows no listening port.
19. **Windows: stdio with the release binary.** Install the `.msi` (a GUI-subsystem
    executable). In a normal terminal run
    `claude mcp add hoprs -- "C:\Program Files\hoprs\hoprs.exe" mcp`, then `/mcp`: hoprs
    connects with its thirteen tools, no console window flashes, and a tool call works. Also
    run the command from a path with a space and with `cmd.exe`, PowerShell and Windows
    Terminal. Check that `hoprs.exe mcp` run by hand in a console waits for input and prints
    nothing else, and that `hoprs.exe --version` still prints in a console.
20. **Windows: local paths.** Ask for a download to a path on another drive and to a UNC
    path: refused with a sentence that says only the current drive is reachable.
21. **Look: English, pt-BR, Turkish.** For each language (Settings › Language) compare with
    mockup v2: the status-bar indicator ("Agent connected", with the tooltip counting
    agents), the "agent" tag on a queue row, the "Connected (by an agent)" mark in the
    sidebar, the line in the security dialog, the site form's section (three segments and
    one hint), and Settings › Agents (MCP) between Terminal and Backup (the switch, the hint,
    the two command lines with Copy, the counts). No shadows, no new modal, the white accent,
    long paths wrap or are cut with the full text on hover. In pt-BR and Turkish no English
    label is left over. @P1LOTRX reviews the Turkish after the merge.
22. **A queue written by this version, opened by hoprs 1.5.0.** With a transfer left in the
    queue (pause one and quit), start hoprs 1.5.0 on the same data: it refuses the queue file
    and runs on an empty in-memory queue. Check that `queue.db` is still in the data folder
    and that this version opens it again. Then, with 1.5.0, open a `sites.toml` that has `agent_access` lines and
    save a site: the line is kept.
23. **Recovery of an agent transfer.** Quit hoprs during a running agent transfer and
    reopen it: the transfer is recovered, tagged "agent", paused until its site reconnects,
    and still never asks about an existing file.
24. **An old relay against the updated window.** Start Claude Code with hoprs registered
    and make one call (the relay is running). Leave the agent session open, update hoprs
    (install a newer build), quit hoprs and open the new one, then make a call from the
    same agent session: the agent gets "This MCP server is older than the hoprs window
    (window <new>, this program <old>)… Restart or reconnect the hoprs MCP server in the
    MCP client…, then call the tool again." Restarting hoprs again changes nothing;
    restarting the agent session (or reconnecting the server in the client) makes the next
    call work.
25. **Windows: an update while a relay runs.** With an agent session open (a `hoprs.exe mcp`
    process running, see Task Manager), update hoprs with the next `.msi` (through the
    update card, or by running the installer by hand). Note what happens: whether the
    installer finishes, asks to close an application or to restart Windows, and whether
    `C:\Program Files\hoprs\hoprs.exe` is the new version afterwards (`hoprs.exe
    --version`). Then end the agent session and check that the update completes (or that a
    second run of the installer does). By reading, the relay keeps `hoprs.exe` in use and
    has no window the installer could close; report what really happens.
