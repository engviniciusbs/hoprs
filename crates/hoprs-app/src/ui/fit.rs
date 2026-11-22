//! What fits at a given window width, down to the 960 px minimum: which file table
//! columns show, which form the pane toolbars use, and whether the queue head is compact.
//! Pure functions of the window width the app last saw ([`crate::app::App::window_width`]).

use std::borrow::Cow;

use crate::backend::Side;
use crate::ui::files::PANES_MIN_HEIGHT;
use crate::ui::layout::{STATUS_HEIGHT, TOP_BAR_HEIGHT};
use crate::ui::theme::HAIRLINE;
use crate::ui::{compare, queue, sidebar};

/// Fixed file table columns; the name takes the rest.
pub const SIZE_WIDTH: f32 = 88.0;
pub const MODIFIED_WIDTH: f32 = 116.0;
pub const PERMISSIONS_WIDTH: f32 = 110.0;
/// The name column never gets less than this while another column can still be dropped
/// (Permissions first, then Modified).
pub const NAME_MIN: f32 = 120.0;

/// From these pane widths the toolbar is [`Toolbar::Full`], icons, labels and key hints
/// (the remote toolbar, with Edit, needs ~610 px; the local one ~460 px, Compare included;
/// every button carries a 16 px icon): both panes of a ~1480 px window. Narrower panes get
/// [`Toolbar::Compact`] (no hints) down to the `TOOLBAR_NARROW_*` widths, then (remote
/// only) [`Toolbar::Trimmed`] down to the `TOOLBAR_TRIMMED_*` widths, then [`Toolbar::Narrow`].
const TOOLBAR_FULL_REMOTE: f32 = 612.0;
/// The same in Portuguese, whose labels are wider ("Renomear", "Nova pasta",
/// "Permissões…").
const TOOLBAR_FULL_REMOTE_PT: f32 = 632.0;
const TOOLBAR_FULL_LOCAL: f32 = 460.0;
/// Below these pane widths the remote toolbar trims the labels of Permissions and Edit,
/// the local one shows icons only (labels and keys in the tooltips). The labelled remote
/// toolbar needs ~505 px (more in Portuguese, ~525 px) and the local one ~365 px,
/// estimated, not measured: each threshold has a margin of 24 px.
const TOOLBAR_NARROW_REMOTE: f32 = 530.0;
const TOOLBAR_NARROW_REMOTE_PT: f32 = 550.0;
const TOOLBAR_NARROW_LOCAL: f32 = 390.0;
/// Below these pane widths even the trimmed remote toolbar (Permissions and Edit without
/// labels, ~410 px; ~425 px in Portuguese) does not fit with its margin: icons only.
const TOOLBAR_TRIMMED_REMOTE: f32 = 435.0;
const TOOLBAR_TRIMMED_REMOTE_PT: f32 = 450.0;
/// Below this center width the queue head uses short labels.
const QUEUE_HEAD_FULL: f32 = 900.0;

/// Advance of JetBrains Mono, in em: exact, every glyph has it.
pub const MONO_ADVANCE: f32 = 0.6;
/// Advance of Inter, in em: an average (the glyphs vary), so the container still clips.
pub const INTER_ADVANCE: f32 = 0.56;

/// `text` cut to fit `width` px at `size` px with a per-character `advance` (em), ending in
/// `…` when it had to be cut; the text itself, borrowed, when it fits. Counts characters.
pub fn ellipsize(text: &str, width: f32, size: f32, advance: f32) -> Cow<'_, str> {
    // The epsilon absorbs f32 rounding (75 / 7.5 must be 10, not 9.999).
    let budget = (width / (size * advance) + 1e-3).floor().max(0.0) as usize;
    let total = text.chars().count();
    if total <= budget {
        return Cow::Borrowed(text);
    }
    let kept: String = text.chars().take(budget.saturating_sub(1)).collect();
    Cow::Owned(format!("{kept}\u{2026}"))
}

/// The compare panel's height in a tab area `area` px tall: `wanted` (the saved one), or
/// [`compare::DEFAULT_SHARE`] of the area; never above what leaves the panes five rows,
/// never below [`compare::MIN_HEIGHT`] (which wins when the window is too small for both).
pub fn compare_height(area: f32, wanted: Option<f32>) -> f32 {
    let wanted = wanted.unwrap_or(area * compare::DEFAULT_SHARE);
    wanted.min(area - PANES_MIN_HEIGHT).max(compare::MIN_HEIGHT)
}

/// The most the queue may take of a `window_height` px window: `wanted` (the saved one),
/// but the panes keep five rows (and the compare panel its minimum, `compare_shown`) next
/// to the log (when `log_open`), the top bar and the status bar. At least
/// [`queue::MIN_HEIGHT`]. The queue still shrinks to its rows below this.
pub fn queue_max(window_height: f32, wanted: f32, log_open: bool, compare_shown: bool) -> f32 {
    let log = if log_open { log_height() } else { 0.0 };
    let compare = if compare_shown {
        compare::MIN_HEIGHT
    } else {
        0.0
    };
    let room = window_height
        - TOP_BAR_HEIGHT
        - HAIRLINE
        - STATUS_HEIGHT
        - log
        - PANES_MIN_HEIGHT
        - compare;
    wanted.min(room).max(queue::MIN_HEIGHT)
}

/// The log panel with its head and rules.
fn log_height() -> f32 {
    crate::ui::layout::QUEUE_HEAD_HEIGHT + 3.0 * HAIRLINE + crate::ui::layout::LOG_BODY_HEIGHT
}

/// The columns a file table shows besides Name and Size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Columns {
    pub modified: bool,
    /// Remote side only.
    pub permissions: bool,
}

/// The width right of the sidebar.
pub fn center_width(window: f32) -> f32 {
    (window - sidebar::WIDTH).max(0.0)
}

/// One of the two equal file panes.
pub fn pane_width(window: f32) -> f32 {
    center_width(window) / 2.0
}

/// The columns that fit in a pane `pane` px wide, keeping at least [`NAME_MIN`] for the
/// name: Permissions goes first, then Modified; Size always stays.
pub fn columns(pane: f32, side: Side) -> Columns {
    let room = pane - NAME_MIN - SIZE_WIDTH;
    let modified = room >= MODIFIED_WIDTH;
    let permissions = side == Side::Remote && room >= MODIFIED_WIDTH + PERMISSIONS_WIDTH;
    Columns {
        modified,
        permissions,
    }
}

/// What a name cell spends besides the name: the 2 px selection bar, both paddings, the
/// kind icon and its gap.
pub const NAME_CHROME: f32 = 2.0 + 2.0 * 16.0 + 16.0 + 8.0;

/// The text width of the name column in a pane `pane` px wide with `cols`: the pane minus
/// the fixed columns and [`NAME_CHROME`]; never below 0.
pub fn name_width(pane: f32, cols: Columns) -> f32 {
    let fixed = SIZE_WIDTH
        + if cols.modified { MODIFIED_WIDTH } else { 0.0 }
        + if cols.permissions {
            PERMISSIONS_WIDTH
        } else {
            0.0
        };
    (pane - fixed - NAME_CHROME).max(0.0)
}

/// How a pane's toolbar reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Toolbar {
    /// Labels and key hints.
    Full,
    /// Labels only.
    Compact,
    /// Labels without hints, except Permissions and Edit: icons with a tooltip (remote).
    Trimmed,
    /// Icons only, labels and keys in the tooltips (the narrowest panes).
    Narrow,
}

impl Toolbar {
    /// Whether the key hints show.
    pub fn hints(self) -> bool {
        self == Self::Full
    }

    /// Whether the least-used actions (Permissions, Edit) are icons only.
    pub fn trimmed(self) -> bool {
        matches!(self, Self::Trimmed | Self::Narrow)
    }

    /// Whether the buttons are icons only.
    pub fn icons_only(self) -> bool {
        self == Self::Narrow
    }
}

/// The toolbar form for a pane `pane` px wide.
pub fn toolbar(pane: f32, side: Side) -> Toolbar {
    // Portuguese and Turkish labels are wider than English ones.
    let pt = matches!(
        crate::i18n::language(),
        crate::i18n::Language::PtBr | crate::i18n::Language::Tr
    );
    let trimmed = match side {
        Side::Local => 0.0,
        Side::Remote if pt => TOOLBAR_TRIMMED_REMOTE_PT,
        Side::Remote => TOOLBAR_TRIMMED_REMOTE,
    };
    let (full, narrow) = match side {
        Side::Local => (
            TOOLBAR_FULL_LOCAL,
            if pt {
                TOOLBAR_NARROW_LOCAL + 15.0
            } else {
                TOOLBAR_NARROW_LOCAL
            },
        ),
        Side::Remote if pt => (TOOLBAR_FULL_REMOTE_PT, TOOLBAR_NARROW_REMOTE_PT),
        Side::Remote => (TOOLBAR_FULL_REMOTE, TOOLBAR_NARROW_REMOTE),
    };
    if pane >= full {
        Toolbar::Full
    } else if pane >= narrow {
        Toolbar::Compact
    } else if pane >= trimmed && side == Side::Remote {
        Toolbar::Trimmed
    } else {
        Toolbar::Narrow
    }
}

/// Whether the queue head uses its short labels at this window width.
pub fn compact_queue(window: f32) -> bool {
    center_width(window) < QUEUE_HEAD_FULL
}

/// How the queue head shows the session's conflict policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolicyNote {
    /// "Conflicts: overwrite (this session)".
    Full,
    /// The short mono tag "overwrite: session".
    Tag,
}

/// What the queue head shows at a window width.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueHead {
    /// Short labels, no worker count, tighter gaps.
    pub compact: bool,
    /// The "Transfers" title.
    pub title: bool,
    /// The session's conflict policy, if one is set.
    pub policy: Option<PolicyNote>,
}

/// The queue head at this window width, with or without a session policy to show. The
/// policy is never hidden: a narrow head shows its tag instead of the title.
pub fn queue_head(window: f32, has_policy: bool) -> QueueHead {
    let compact = compact_queue(window);
    let policy = has_policy.then_some(if compact {
        PolicyNote::Tag
    } else {
        PolicyNote::Full
    });
    QueueHead {
        compact,
        title: policy != Some(PolicyNote::Tag),
        policy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compare_default_is_thirty_percent() {
        assert_eq!(compare_height(900.0, None), 270.0);
    }

    #[test]
    fn compare_clamps_to_its_minimum_and_the_panes() {
        assert_eq!(compare_height(900.0, Some(800.0)), 900.0 - PANES_MIN_HEIGHT);
        assert_eq!(compare_height(300.0, None), compare::MIN_HEIGHT);
        assert_eq!(compare_height(300.0, Some(800.0)), compare::MIN_HEIGHT);
        assert_eq!(compare_height(900.0, Some(1.0)), compare::MIN_HEIGHT);
        assert_eq!(compare_height(900.0, Some(240.5)), 240.5);
    }

    #[test]
    fn queue_max_clamps() {
        let chrome = TOP_BAR_HEIGHT + HAIRLINE + STATUS_HEIGHT;
        // Room enough: the saved value.
        assert_eq!(queue_max(1200.0, 300.0, false, false), 300.0);
        // 600 at 800 px leaves the panes their five rows.
        assert_eq!(
            queue_max(800.0, 600.0, false, false),
            800.0 - chrome - PANES_MIN_HEIGHT
        );
        // The log and a compare panel take their room first.
        assert_eq!(
            queue_max(800.0, 600.0, false, true),
            800.0 - chrome - PANES_MIN_HEIGHT - compare::MIN_HEIGHT
        );
        assert!(queue_max(800.0, 600.0, true, false) < queue_max(800.0, 600.0, false, false));
        // A tiny window: the queue's minimum.
        assert_eq!(queue_max(300.0, 600.0, true, true), queue::MIN_HEIGHT);
    }

    const ALL: Columns = Columns {
        modified: true,
        permissions: true,
    };

    #[test]
    fn ellipsize_keeps_text_that_fits() {
        let out = ellipsize("short.txt", 200.0, 12.5, MONO_ADVANCE);
        assert!(matches!(out, Cow::Borrowed("short.txt")));
    }

    #[test]
    fn ellipsize_cuts_with_an_ellipsis() {
        // 75 / (12.5 * 0.6) = 10 characters.
        let out = ellipsize("public_html_backup_2026", 75.0, 12.5, MONO_ADVANCE);
        assert_eq!(out.chars().count(), 10);
        assert!(out.ends_with('\u{2026}'));
        assert!(out.starts_with("public_ht"));
    }

    #[test]
    fn ellipsize_counts_chars_not_bytes() {
        // 4 characters, 6 bytes; a budget of 4 keeps all of them.
        let width = 4.0 * 10.0 * MONO_ADVANCE;
        assert!(matches!(
            ellipsize("a\u{e7}\u{e3}o", width, 10.0, MONO_ADVANCE),
            Cow::Borrowed(_)
        ));
        let out = ellipsize("a\u{e7}\u{e3}o!!", width, 10.0, MONO_ADVANCE);
        assert_eq!(out, "a\u{e7}\u{e3}\u{2026}");
    }

    #[test]
    fn ellipsize_zero_width_is_just_the_ellipsis() {
        assert_eq!(ellipsize("anything", 0.0, 12.5, MONO_ADVANCE), "\u{2026}");
        assert_eq!(ellipsize("anything", 5.0, 12.5, INTER_ADVANCE), "\u{2026}");
    }

    #[test]
    fn name_width_shrinks_with_columns() {
        let all = name_width(600.0, ALL);
        let some = name_width(
            600.0,
            Columns {
                modified: true,
                permissions: false,
            },
        );
        let few = name_width(
            600.0,
            Columns {
                modified: false,
                permissions: false,
            },
        );
        assert!(all < some && some < few, "{all} {some} {few}");
        assert_eq!(
            all,
            600.0 - SIZE_WIDTH - MODIFIED_WIDTH - PERMISSIONS_WIDTH - NAME_CHROME
        );
        assert_eq!(name_width(10.0, ALL), 0.0);
    }

    #[test]
    fn columns_drop_permissions_then_modified() {
        // 1280 px window: both panes ~512 px, everything fits.
        assert_eq!(columns(pane_width(1280.0), Side::Remote), ALL);
        assert_eq!(
            columns(pane_width(1280.0), Side::Local),
            Columns {
                modified: true,
                permissions: false
            }
        );
        // 960 px (the minimum): 352 px panes; the remote side drops Permissions only.
        let narrow = pane_width(960.0);
        assert_eq!(narrow, 352.0);
        assert_eq!(
            columns(narrow, Side::Remote),
            Columns {
                modified: true,
                permissions: false
            }
        );
        assert!(columns(narrow, Side::Local).modified);
        // The exact thresholds.
        let full = NAME_MIN + SIZE_WIDTH + MODIFIED_WIDTH + PERMISSIONS_WIDTH;
        assert_eq!(columns(full, Side::Remote), ALL);
        assert!(!columns(full - 1.0, Side::Remote).permissions);
        let with_modified = NAME_MIN + SIZE_WIDTH + MODIFIED_WIDTH;
        assert!(columns(with_modified, Side::Local).modified);
        assert_eq!(
            columns(with_modified - 1.0, Side::Remote),
            Columns {
                modified: false,
                permissions: false
            }
        );
    }

    #[test]
    fn name_keeps_its_minimum_while_columns_can_drop() {
        for pane in (200..1200).map(|w| w as f32) {
            let c = columns(pane, Side::Remote);
            let fixed = SIZE_WIDTH
                + if c.modified { MODIFIED_WIDTH } else { 0.0 }
                + if c.permissions {
                    PERMISSIONS_WIDTH
                } else {
                    0.0
                };
            if c.modified {
                assert!(pane - fixed >= NAME_MIN, "{pane}: {c:?}");
            }
        }
    }

    /// The Portuguese labels are wider: the remote toolbar needs more room for its hints
    /// and for its labels (Compact), each with a margin.
    #[test]
    fn remote_toolbar_needs_more_room_in_portuguese() {
        crate::i18n::set_language(crate::i18n::Language::PtBr);
        assert_eq!(
            toolbar(TOOLBAR_FULL_REMOTE_PT - 1.0, Side::Remote),
            Toolbar::Compact
        );
        assert_eq!(toolbar(TOOLBAR_FULL_REMOTE_PT, Side::Remote), Toolbar::Full);
        assert_eq!(
            toolbar(TOOLBAR_NARROW_REMOTE_PT - 1.0, Side::Remote),
            Toolbar::Trimmed
        );
        assert_eq!(
            toolbar(TOOLBAR_TRIMMED_REMOTE_PT, Side::Remote),
            Toolbar::Trimmed
        );
        assert_eq!(
            toolbar(TOOLBAR_TRIMMED_REMOTE_PT - 1.0, Side::Remote),
            Toolbar::Narrow
        );
        // The 512 px pane of the default window keeps the main labels in Portuguese too.
        assert_eq!(toolbar(512.0, Side::Remote), Toolbar::Trimmed);
        assert_eq!(toolbar(512.0, Side::Local), Toolbar::Full);
        assert_eq!(
            toolbar(TOOLBAR_NARROW_REMOTE_PT, Side::Remote),
            Toolbar::Compact
        );
        // English has its own, smaller boundaries.
        crate::i18n::set_language(crate::i18n::Language::En);
        assert_eq!(
            toolbar(TOOLBAR_NARROW_REMOTE_PT - 1.0, Side::Remote),
            Toolbar::Compact
        );
        assert_eq!(
            toolbar(TOOLBAR_FULL_REMOTE_PT - 1.0, Side::Remote),
            Toolbar::Full
        );
    }

    #[test]
    fn toolbars_and_queue_head_go_compact_when_narrow() {
        // Every button has a 16 px icon, so the hints need wider panes than before.
        assert_eq!(toolbar(pane_width(1480.0), Side::Remote), Toolbar::Full);
        assert_eq!(toolbar(pane_width(1440.0), Side::Remote), Toolbar::Compact);
        assert_eq!(
            toolbar(TOOLBAR_NARROW_REMOTE, Side::Remote),
            Toolbar::Compact
        );
        assert_eq!(
            toolbar(TOOLBAR_NARROW_REMOTE - 1.0, Side::Remote),
            Toolbar::Trimmed
        );
        assert_eq!(
            toolbar(TOOLBAR_TRIMMED_REMOTE - 1.0, Side::Remote),
            Toolbar::Narrow
        );
        assert_eq!(toolbar(pane_width(1180.0), Side::Local), Toolbar::Full);
        assert_eq!(toolbar(TOOLBAR_NARROW_LOCAL, Side::Local), Toolbar::Compact);
        assert_eq!(
            toolbar(TOOLBAR_NARROW_LOCAL - 1.0, Side::Local),
            Toolbar::Narrow
        );
        // The default 1280 px window (512 px panes): the remote side keeps the main labels
        // (only Permissions and Edit are icons), with the margin the form needs.
        assert_eq!(toolbar(pane_width(1280.0), Side::Remote), Toolbar::Trimmed);
        assert_eq!(toolbar(pane_width(1280.0), Side::Local), Toolbar::Full);
        // At the 960 px minimum (352 px panes) both toolbars are icons only.
        assert_eq!(toolbar(pane_width(960.0), Side::Remote), Toolbar::Narrow);
        assert_eq!(toolbar(pane_width(960.0), Side::Local), Toolbar::Narrow);
        assert!(!Toolbar::Narrow.hints() && !Toolbar::Compact.hints() && Toolbar::Full.hints());
        assert!(Toolbar::Narrow.icons_only() && !Toolbar::Compact.icons_only());
        assert!(!compact_queue(1280.0));
        assert!(compact_queue(960.0));
        assert_eq!(center_width(100.0), 0.0);
    }

    /// The session's conflict policy never disappears from the queue head: a narrow
    /// window shows it as a short mono tag, making room by dropping the title.
    #[test]
    fn queue_head_keeps_the_session_policy_when_narrow() {
        let wide = queue_head(1280.0, true);
        assert_eq!(
            wide,
            QueueHead {
                compact: false,
                title: true,
                policy: Some(PolicyNote::Full)
            }
        );
        let narrow = queue_head(960.0, true);
        assert_eq!(
            narrow,
            QueueHead {
                compact: true,
                title: false,
                policy: Some(PolicyNote::Tag)
            }
        );
        // The threshold: the full note while the head is not compact.
        let edge = sidebar::WIDTH + QUEUE_HEAD_FULL;
        assert_eq!(queue_head(edge, true).policy, Some(PolicyNote::Full));
        assert_eq!(queue_head(edge - 1.0, true).policy, Some(PolicyNote::Tag));
        // No policy: nothing to show, and the title stays.
        for width in [960.0, 1280.0] {
            let head = queue_head(width, false);
            assert_eq!((head.title, head.policy), (true, None));
        }
        assert_eq!(
            crate::ui::strings::session_policy_tag(hoprs_transfer::ConflictPolicy::Overwrite),
            "overwrite: session"
        );
        assert_eq!(
            crate::ui::strings::session_policy_tag(
                hoprs_transfer::ConflictPolicy::OverwriteIfNewer
            ),
            "if newer: session"
        );
    }
}
