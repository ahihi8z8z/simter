/**
 * Turns off the terminal modes a remote program may have left on, written before a new ssh
 * process starts in the same terminal. When the previous connection was killed or dropped, the
 * program (e.g. tmux with `mouse on`) never got to undo them, so the new login shell would
 * receive mouse reports as text such as `35;18;5M` on every mouse move.
 *
 * - `CSI ? 1049 l` leaves the alternate screen, back to the scrollback.
 * - `CSI ! p` (DECSTR) resets cursor visibility and style, scroll region, colors, charsets,
 *   application cursor keys/keypad, bracketed paste and focus reporting.
 * - `CSI ? … l` turns off mouse reporting and its encodings, which DECSTR leaves alone.
 */
export const RESET_REMOTE_MODES = "\x1b[?1049l\x1b[!p\x1b[?9;1000;1001;1002;1003;1005;1006;1015;1016l";
