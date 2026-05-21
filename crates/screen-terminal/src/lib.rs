#![forbid(unsafe_code)]

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub columns: u16,
    pub rows: u16,
}

impl Dimensions {
    pub const fn new(columns: u16, rows: u16) -> Self {
        Self { columns, rows }
    }

    fn normalized(self) -> Self {
        Self {
            columns: self.columns.max(1),
            rows: self.rows.max(1),
        }
    }

    fn area(self) -> usize {
        usize::from(self.columns) * usize::from(self.rows)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cursor {
    pub column: u16,
    pub row: u16,
}

// ---------------------------------------------------------------------------
// Color
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Color {
    Basic(u8),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

// ---------------------------------------------------------------------------
// Style
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub blink: bool,
    pub reverse: bool,
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

// ---------------------------------------------------------------------------
// Cell
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cell {
    pub bytes: Vec<u8>,
    pub style: Style,
}

impl Cell {
    pub fn blank(style: Style) -> Self {
        Self {
            bytes: Vec::new(),
            style,
        }
    }

    pub fn is_blank(&self) -> bool {
        self.bytes.is_empty()
    }
}

// ---------------------------------------------------------------------------
// Grid
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct Grid {
    cells: Vec<Cell>,
    columns: u16,
    rows: u16,
}

impl Grid {
    fn new(dimensions: Dimensions, style: Style) -> Self {
        let area = dimensions.area();
        Self {
            cells: (0..area).map(|_| Cell::blank(style)).collect(),
            columns: dimensions.columns,
            rows: dimensions.rows,
        }
    }

    fn area(&self) -> usize {
        self.cells.len()
    }

    fn index(&self, column: u16, row: u16) -> usize {
        usize::from(row) * usize::from(self.columns) + usize::from(column)
    }

    fn cell(&self, column: u16, row: u16) -> Option<&Cell> {
        if column >= self.columns || row >= self.rows {
            return None;
        }
        self.cells.get(self.index(column, row))
    }

    fn set_cell(&mut self, column: u16, row: u16, cell: Cell) {
        let idx = self.index(column, row);
        if idx < self.cells.len() {
            self.cells[idx] = cell;
        }
    }

    fn clear_range(&mut self, start: usize, end: usize, style: Style) {
        for index in start.min(self.area())..end.min(self.area()) {
            self.cells[index] = Cell::blank(style);
        }
    }

    /// Scroll the region between `top` and `bottom` (inclusive) up by `count` lines.
    fn scroll_up(&mut self, top: u16, bottom: u16, count: u16, style: Style) {
        let count = count.min(bottom.saturating_sub(top).saturating_add(1));
        if count == 0 {
            return;
        }
        let cols = usize::from(self.columns);
        let region_start = usize::from(top) * cols;
        let region_end = usize::from(bottom + 1) * cols;
        let remove_len = usize::from(count) * cols;

        // Remove `count` rows from the top of the region
        let drain_end = (region_start + remove_len).min(region_end);
        self.cells.drain(region_start..drain_end);
        // Insert `count` blank rows at the bottom of the region
        // After drain, the region_end has shifted left by `remove_len`
        let insert_pos = region_end.saturating_sub(remove_len);
        let insert_pos = insert_pos.min(self.cells.len());
        for _ in 0..remove_len {
            self.cells.insert(insert_pos, Cell::blank(style));
        }
    }

    /// Scroll the region between `top` and `bottom` (inclusive) down by `count` lines.
    fn scroll_down(&mut self, top: u16, bottom: u16, count: u16, style: Style) {
        let count = count.min(bottom.saturating_sub(top).saturating_add(1));
        if count == 0 {
            return;
        }
        let cols = usize::from(self.columns);
        let region_start = usize::from(top) * cols;
        let region_end = usize::from(bottom + 1) * cols;
        let remove_len = usize::from(count) * cols;

        // Remove `count` rows from the bottom of the region
        let drain_start = region_end.saturating_sub(remove_len);
        let drain_end = region_end.min(self.cells.len());
        self.cells.drain(drain_start..drain_end);
        // Insert `count` blank rows at the top of the region
        for _ in 0..remove_len {
            self.cells.insert(region_start, Cell::blank(style));
        }
    }

    /// Insert `count` blank lines at `row`, scrolling the region downward.
    /// Lines that fall off the bottom of the region are lost.
    fn insert_lines(&mut self, row: u16, bottom: u16, count: u16, style: Style) {
        self.scroll_down(row, bottom, count, style);
    }

    /// Delete `count` lines starting at `row`, scrolling the region upward.
    /// Blank lines fill the vacated space at the bottom.
    fn delete_lines(&mut self, row: u16, bottom: u16, count: u16, style: Style) {
        self.scroll_up(row, bottom, count, style);
    }

    /// Insert `count` blank cells at (col, row), shifting existing cells right.
    /// Cells that fall off the right edge are lost.
    fn insert_cells(&mut self, column: u16, row: u16, count: u16, style: Style) {
        let count = count.min(self.columns - column);
        if count == 0 {
            return;
        }
        for shift_col in (column..self.columns - count).rev() {
            if let Some(src) = self.cell(shift_col, row).cloned() {
                self.set_cell(shift_col + count, row, src);
            }
        }
        for col in column..column + count {
            self.set_cell(col, row, Cell::blank(style));
        }
    }

    /// Delete `count` cells at (col, row), shifting remaining cells left.
    /// Blank cells fill the vacated space at the right.
    fn delete_cells(&mut self, column: u16, row: u16, count: u16, style: Style) {
        let count = count.min(self.columns - column);
        if count == 0 {
            return;
        }
        for col in column..self.columns - count {
            if let Some(src) = self.cell(col + count, row).cloned() {
                self.set_cell(col, row, src);
            }
        }
        for col in self.columns - count..self.columns {
            self.set_cell(col, row, Cell::blank(style));
        }
    }

    fn line_bytes(&self, row: u16) -> Option<Vec<u8>> {
        if row >= self.rows {
            return None;
        }
        let mut bytes = Vec::new();
        for column in 0..self.columns {
            let cell = &self.cells[self.index(column, row)];
            if cell.is_blank() {
                bytes.push(b' ');
            } else {
                bytes.extend_from_slice(&cell.bytes);
            }
        }
        while bytes.last() == Some(&b' ') {
            bytes.pop();
        }
        Some(bytes)
    }
}

// ---------------------------------------------------------------------------
// Margins (scrolling region)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Margins {
    top: u16,
    bottom: u16,
}

impl Margins {
    fn full(rows: u16) -> Self {
        Self {
            top: 0,
            bottom: rows.saturating_sub(1),
        }
    }
}

// ---------------------------------------------------------------------------
// Modes
// ---------------------------------------------------------------------------

/// Cursor style for DECSCUSR.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CursorStyle {
    #[default]
    BlinkingBlock,
    SteadyBlock,
    BlinkingUnderline,
    SteadyUnderline,
    BlinkingBar,
    SteadyBar,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Modes {
    /// Origin mode (DECOM): cursor positioning is relative to margins.
    origin: bool,
    /// Insert/replace mode (IRM).
    insert: bool,
    /// Application cursor keys (DECCKM).
    application_cursor: bool,
    /// Application keypad (DECKPAM).
    application_keypad: bool,
    /// Bracketed paste.
    bracketed_paste: bool,
    /// Mouse tracking mode (SGR extended / 1006).
    mouse_sgr: bool,
    /// Auto-wrap (DECAWM).
    auto_wrap: bool,
    /// Reverse video (DECSCNM).
    reverse_screen: bool,
    /// Show/hide cursor (DECTCEM).
    show_cursor: bool,
    /// Cursor shape (DECSCUSR).
    cursor_style: CursorStyle,
}

impl Modes {
    fn default_on() -> Self {
        Self {
            auto_wrap: true,
            show_cursor: true,
            cursor_style: CursorStyle::BlinkingBlock,
            ..Default::default()
        }
    }
}

// ---------------------------------------------------------------------------
// TerminalState
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalState {
    pub dimensions: Dimensions,
    pub cursor: Cursor,
    pub title: Option<Vec<u8>>,
    primary: Grid,
    alternate: Option<Grid>,
    using_alternate: bool,
    current_style: Style,
    saved_cursor: Cursor,
    saved_style: Style,
    modes: Modes,
    margins: Margins,
    parser: ParserState,
    scrollback: Vec<Vec<Cell>>,
    scrollback_max: u32,
    /// Accumulated escape sequence responses to write back to pty.
    response_buffer: Vec<u8>,
}

impl TerminalState {
    pub fn new(dimensions: Dimensions) -> Self {
        let dimensions = dimensions.normalized();
        let cursor = Cursor { column: 0, row: 0 };
        let style = Style::default();
        Self {
            dimensions,
            cursor,
            title: None,
            primary: Grid::new(dimensions, style),
            alternate: None,
            using_alternate: false,
            current_style: style,
            saved_cursor: cursor,
            saved_style: style,
            modes: Modes::default_on(),
            margins: Margins::full(dimensions.rows),
            parser: ParserState::Ground,
            scrollback: Vec::new(),
            scrollback_max: 1000,
            response_buffer: Vec::new(),
        }
    }

    // -- public API ----------------------------------------------------------

    /// Set the maximum number of scrollback lines.
    pub fn set_scrollback_limit(&mut self, max: u32) {
        self.scrollback_max = max;
        // Trim excess
        while self.scrollback.len() > max as usize {
            self.scrollback.remove(0);
        }
    }

    /// Number of scrollback lines available.
    pub fn scrollback_len(&self) -> usize {
        self.scrollback.len()
    }

    /// Get a line from the scrollback buffer (0 = most recent, i.e., the line that just scrolled off).
    pub fn scrollback_line(&self, index: usize) -> Option<Vec<u8>> {
        if index >= self.scrollback.len() {
            return None;
        }
        let line = &self.scrollback[self.scrollback.len() - 1 - index];
        let mut bytes = Vec::new();
        for cell in line {
            if cell.is_blank() {
                bytes.push(b' ');
            } else {
                bytes.extend_from_slice(&cell.bytes);
            }
        }
        while bytes.last() == Some(&b' ') {
            bytes.pop();
        }
        Some(bytes)
    }

    /// Get a specific scrollback cell (for copy mode cursor navigation).
    pub fn scrollback_cell(&self, row: usize, col: u16) -> Option<&Cell> {
        if row >= self.scrollback.len() || col >= self.dimensions.columns {
            return None;
        }
        let line = &self.scrollback[self.scrollback.len() - 1 - row];
        line.get(col as usize)
    }

    /// Total visible + scrollback rows (for copy mode bounds).
    pub fn total_rows(&self) -> usize {
        self.scrollback.len() + usize::from(self.dimensions.rows)
    }

    /// Get a cell from either scrollback or the visible grid.
    /// Row 0 = top of scrollback, row > scrollback_len = visible grid.
    pub fn cell_at(&self, row: usize, col: u16) -> Option<&Cell> {
        if col >= self.dimensions.columns {
            return None;
        }
        let sb_len = self.scrollback.len();
        if row < sb_len {
            self.scrollback_cell(sb_len - 1 - row, col)
        } else {
            let vis_row = (row - sb_len) as u16;
            self.grid().cell(col, vis_row)
        }
    }

    pub fn apply(&mut self, bytes: &[u8]) -> Vec<u8> {
        for byte in bytes {
            self.apply_byte(*byte);
        }
        std::mem::take(&mut self.response_buffer)
    }

    pub fn cell(&self, column: u16, row: u16) -> Option<&Cell> {
        self.grid().cell(column, row)
    }

    pub fn line_bytes(&self, row: u16) -> Option<Vec<u8>> {
        self.grid().line_bytes(row)
    }

    pub fn plain_text(&self) -> String {
        let grid = self.grid();
        (0..grid.rows)
            .map(|row| {
                String::from_utf8_lossy(&grid.line_bytes(row).unwrap_or_default()).into_owned()
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn current_style(&self) -> Style {
        self.current_style
    }

    /// Resize the terminal, growing or shrinking grids.
    pub fn resize(&mut self, dimensions: Dimensions) {
        let dimensions = dimensions.normalized();
        self.dimensions = dimensions;
        self.margins = Margins::full(dimensions.rows);
        self.cursor = self.clamp_cursor(self.cursor);
        // Re-wrap scrollback lines to match new width
        self.rewrap_scrollback(dimensions.columns);
    }

    fn rewrap_scrollback(&mut self, new_cols: u16) {
        if new_cols == 0 {
            return;
        }
        let mut rewrapped: Vec<Vec<Cell>> = Vec::with_capacity(self.scrollback.len());
        for line in self.scrollback.drain(..) {
            if line.len() <= new_cols as usize {
                // Pad shorter lines
                let mut padded = line;
                while padded.len() < new_cols as usize {
                    padded.push(Cell::blank(Style::default()));
                }
                rewrapped.push(padded);
            } else {
                // Split longer lines (naive: just trim)
                rewrapped.push(line[..new_cols as usize].to_vec());
            }
        }
        self.scrollback = rewrapped;
        // Trim to limit
        while self.scrollback.len() > self.scrollback_max as usize {
            self.scrollback.remove(0);
        }
    }

    // -- internal ------------------------------------------------------------

    fn grid(&self) -> &Grid {
        if self.using_alternate {
            self.alternate.as_ref().unwrap_or(&self.primary)
        } else {
            &self.primary
        }
    }

    fn grid_mut(&mut self) -> &mut Grid {
        if self.using_alternate {
            self.alternate.as_mut().unwrap_or(&mut self.primary)
        } else {
            &mut self.primary
        }
    }

    fn apply_byte(&mut self, byte: u8) {
        let parser = std::mem::replace(&mut self.parser, ParserState::Ground);
        self.parser = match parser {
            ParserState::Ground => self.apply_ground(byte),
            ParserState::Escape => self.apply_escape(byte),
            ParserState::Csi(mut csi) => self.apply_csi_byte(&mut csi, byte),
            ParserState::Osc(mut osc) => self.apply_osc_byte(&mut osc, byte),
        };
    }

    // -- ground state --------------------------------------------------------

    fn apply_ground(&mut self, byte: u8) -> ParserState {
        match byte {
            b'\x1b' => ParserState::Escape,
            b'\r' => {
                self.cursor.column = 0;
                ParserState::Ground
            }
            b'\n' | 0x0b | 0x0c => {
                self.line_feed();
                ParserState::Ground
            }
            b'\x08' => {
                self.cursor.column = self.cursor.column.saturating_sub(1);
                ParserState::Ground
            }
            b'\t' => {
                self.horizontal_tab();
                ParserState::Ground
            }
            0x20..=0x7e | 0xa0..=0xff => {
                self.put_byte(byte);
                ParserState::Ground
            }
            _ => ParserState::Ground,
        }
    }

    // -- escape state --------------------------------------------------------

    fn apply_escape(&mut self, byte: u8) -> ParserState {
        match byte {
            b'[' => ParserState::Csi(CsiState::default()),
            b']' => ParserState::Osc(OscState::default()),
            b'(' => ParserState::Ground, // G0 charset select – ignored for now
            b')' => ParserState::Ground, // G1 charset select
            b'7' => {
                self.saved_cursor = self.cursor;
                self.saved_style = self.current_style;
                ParserState::Ground
            }
            b'8' => {
                self.cursor = self.clamp_cursor(self.saved_cursor);
                self.current_style = self.saved_style;
                ParserState::Ground
            }
            b'c' => {
                // RIS – reset to initial state
                let dimensions = self.dimensions;
                *self = Self::new(dimensions);
                ParserState::Ground
            }
            b'H' => {
                // HTS – set tab stop at current column
                ParserState::Ground
            }
            // DECALN – alignment test (fill with 'E') – ignored for now
            b'#' => ParserState::Ground,
            _ => ParserState::Ground,
        }
    }

    // -- CSI state -----------------------------------------------------------

    fn apply_csi_byte(&mut self, csi: &mut CsiState, byte: u8) -> ParserState {
        match byte {
            b'?' if csi.params.is_empty() && csi.current.is_none() => {
                csi.private = true;
                ParserState::Csi(csi.clone())
            }
            b'>' if csi.params.is_empty() && csi.current.is_none() => {
                // DA2, DA3 prefix – ignored; just consume
                ParserState::Csi(csi.clone())
            }
            b'0'..=b'9' => {
                let digit = u16::from(byte - b'0');
                let value = csi
                    .current
                    .unwrap_or(0)
                    .saturating_mul(10)
                    .saturating_add(digit);
                csi.current = Some(value);
                ParserState::Csi(csi.clone())
            }
            b';' => {
                csi.params.push(csi.current.take());
                ParserState::Csi(csi.clone())
            }
            // Intermediate bytes (before final)
            0x20..=0x2f => {
                csi.intermediates.push(byte);
                ParserState::Csi(csi.clone())
            }
            // Final byte
            final_byte @ 0x40..=0x7e => {
                csi.params.push(csi.current.take());
                if csi.private {
                    self.execute_dec_csi(final_byte, &csi.params);
                } else {
                    self.execute_csi(final_byte, &csi.params, &csi.intermediates);
                }
                ParserState::Ground
            }
            _ => ParserState::Csi(csi.clone()),
        }
    }

    // -- OSC state -----------------------------------------------------------

    fn apply_osc_byte(&mut self, osc: &mut OscState, byte: u8) -> ParserState {
        if osc.escape_seen {
            osc.escape_seen = false;
            if byte == b'\\' {
                self.finish_osc(&osc.bytes);
                return ParserState::Ground;
            }
            osc.push(b'\x1b');
        }

        match byte {
            b'\x07' => {
                self.finish_osc(&osc.bytes);
                ParserState::Ground
            }
            b'\x1b' => {
                osc.escape_seen = true;
                ParserState::Osc(osc.clone())
            }
            _ => {
                osc.push(byte);
                ParserState::Osc(osc.clone())
            }
        }
    }

    // -- CSI execution -------------------------------------------------------

    fn execute_csi(&mut self, command: u8, params: &[Option<u16>], intermediates: &[u8]) {
        match command {
            b'@' => self.insert_characters(param_or(params, 0, 1)),
            b'A' => self.move_vertical(-i32::from(param_or(params, 0, 1))),
            b'B' => self.move_vertical(i32::from(param_or(params, 0, 1))),
            b'C' => self.move_horizontal(i32::from(param_or(params, 0, 1))),
            b'D' => self.move_horizontal(-i32::from(param_or(params, 0, 1))),
            b'E' => self.cursor_next_line(param_or(params, 0, 1)),
            b'F' => self.cursor_prev_line(param_or(params, 0, 1)),
            b'G' => self.cursor_horizontal_absolute(param_or(params, 0, 1)),
            b'H' | b'f' => self.cursor_position(params),
            b'J' => self.erase_display(param_or(params, 0, 0)),
            b'K' => self.erase_line(param_or(params, 0, 0)),
            b'L' => self.insert_lines(param_or(params, 0, 1)),
            b'M' => self.delete_lines(param_or(params, 0, 1)),
            b'P' => self.delete_characters(param_or(params, 0, 1)),
            b'S' => self.scroll_up_csi(param_or(params, 0, 1)),
            b'T' => self.scroll_down_csi(param_or(params, 0, 1)),
            b'X' => self.erase_characters(param_or(params, 0, 1)),
            b'd' => self.cursor_line_absolute(param_or(params, 0, 1)),
            b'h' => self.set_mode(params, intermediates),
            b'l' => self.reset_mode(params, intermediates),
            b'm' => self.apply_sgr(params),
            b'q' if intermediates.contains(&b' ') => {
                // DECSCUSR – cursor style
                let style = param_or(params, 0, 0);
                self.modes.cursor_style = match style {
                    0 | 1 => CursorStyle::BlinkingBlock,
                    2 => CursorStyle::SteadyBlock,
                    3 => CursorStyle::BlinkingUnderline,
                    4 => CursorStyle::SteadyUnderline,
                    5 => CursorStyle::BlinkingBar,
                    6 => CursorStyle::SteadyBar,
                    _ => self.modes.cursor_style,
                };
            }
            b'n' => self.device_status_report(params),
            b'r' => self.set_margins(params),
            b's' => {
                // Save cursor (ANSI.SYS style)
                self.saved_cursor = self.cursor;
                self.saved_style = self.current_style;
            }
            b'u' => {
                // Restore cursor (ANSI.SYS style)
                self.cursor = self.clamp_cursor(self.saved_cursor);
                self.current_style = self.saved_style;
            }
            b'c' => {
                if intermediates.contains(&b'>') {
                    // Secondary DA (CSI > c) – respond as GNU Screen
                    self.response_buffer.extend_from_slice(b"\x1b[>41;304;0c");
                } else {
                    // Primary DA – respond as VT220
                    self.response_buffer
                        .extend_from_slice(b"\x1b[?62;1;2;6;7;8;9;15;22c");
                }
            }
            _ => {}
        }
    }

    /// DEC private mode set/reset (CSI ? ... h / l)
    fn execute_dec_csi(&mut self, command: u8, params: &[Option<u16>]) {
        match command {
            b'h' => {
                for p in params {
                    let value = p.unwrap_or(0);
                    match value {
                        1 => self.modes.application_cursor = true, // DECCKM
                        6 => self.modes.origin = true,             // DECOM
                        7 => self.modes.auto_wrap = true,          // DECAWM
                        9 => {}                                    // X10 mouse – ignored
                        12 => {}                                   // send/receive (SRM) – ignored
                        25 => self.modes.show_cursor = true,       // DECTCEM
                        47 => self.use_alternate_screen(true),     // alt screen
                        1000 => {}                                 // xterm mouse tracking – ignored
                        1002 => {}                                 // cell motion tracking – ignored
                        1003 => {}                                 // all motion tracking – ignored
                        1004 => {}                                 // focus tracking – ignored
                        1005 => {}                                 // utf-8 mouse – ignored
                        1006 => self.modes.mouse_sgr = true,       // SGR mouse
                        1047 => self.use_alternate_screen(true),   // alt screen (xterm)
                        1048 => {
                            // Save cursor (associated with 1047/1049)
                            self.saved_cursor = self.cursor;
                            self.saved_style = self.current_style;
                        }
                        1049 => {
                            // Save cursor + switch to alt screen
                            self.saved_cursor = self.cursor;
                            self.saved_style = self.current_style;
                            self.use_alternate_screen(true);
                        }
                        2004 => self.modes.bracketed_paste = true,
                        _ => {}
                    }
                }
            }
            b'l' => {
                for p in params {
                    let value = p.unwrap_or(0);
                    match value {
                        1 => self.modes.application_cursor = false, // DECCKM
                        6 => self.modes.origin = false,             // DECOM
                        7 => self.modes.auto_wrap = false,          // DECAWM
                        25 => self.modes.show_cursor = false,       // DECTCEM
                        47 => self.use_alternate_screen(false),     // alt screen
                        1000 => {}
                        1002 => {}
                        1003 => {}
                        1004 => {}
                        1005 => {}
                        1006 => self.modes.mouse_sgr = false,
                        1047 => self.use_alternate_screen(false),
                        1049 => self.use_alternate_screen(false),
                        2004 => self.modes.bracketed_paste = false,
                        _ => {}
                    }
                }
            }
            b'r' => {
                // DECSTBM (with ? prefix, some terminals accept)
                self.set_margins(params);
            }
            b'n' => {
                // DECDSR
                self.device_status_report(params);
            }
            _ => {}
        }
    }

    // -- cursor movement helpers --------------------------------------------

    fn cursor_position(&mut self, params: &[Option<u16>]) {
        let row = param_or(params, 0, 1).saturating_sub(1);
        let column = param_or(params, 1, 1).saturating_sub(1);
        if self.modes.origin {
            self.cursor = self.clamp_cursor(Cursor {
                column,
                row: row.saturating_add(self.margins.top),
            });
        } else {
            self.cursor = self.clamp_cursor(Cursor { column, row });
        }
    }

    fn cursor_next_line(&mut self, count: u16) {
        self.cursor.column = 0;
        for _ in 0..count {
            self.line_feed();
        }
    }

    fn cursor_prev_line(&mut self, count: u16) {
        self.cursor.column = 0;
        self.move_vertical(-i32::from(count));
    }

    fn cursor_horizontal_absolute(&mut self, column: u16) {
        self.cursor.column = column.saturating_sub(1).min(self.dimensions.columns - 1);
    }

    fn cursor_line_absolute(&mut self, row: u16) {
        let row = row.saturating_sub(1);
        if self.modes.origin {
            self.cursor.row = (self.margins.top.saturating_add(row)).min(self.margins.bottom);
        } else {
            self.cursor.row = row.min(self.dimensions.rows - 1);
        }
    }

    // -- mode handling -------------------------------------------------------

    fn set_mode(&mut self, params: &[Option<u16>], _intermediates: &[u8]) {
        for p in params {
            if p.unwrap_or(0) == 4 {
                self.modes.insert = true; // IRM
            }
        }
    }

    fn reset_mode(&mut self, params: &[Option<u16>], _intermediates: &[u8]) {
        for p in params {
            if p.unwrap_or(0) == 4 {
                self.modes.insert = false; // IRM
            }
        }
    }

    fn device_status_report(&mut self, params: &[Option<u16>]) {
        match param_or(params, 0, 0) {
            5 => {
                self.response_buffer.extend_from_slice(b"\x1b[0n");
            }
            6 => {
                let row = self.cursor.row.saturating_add(1);
                let col = self.cursor.column.saturating_add(1);
                self.response_buffer.extend_from_slice(b"\x1b[");
                write_uint(&mut self.response_buffer, row as u64);
                self.response_buffer.push(b';');
                write_uint(&mut self.response_buffer, col as u64);
                self.response_buffer.push(b'R');
            }
            _ => {}
        }
    }

    // -- margins (scrolling region) ------------------------------------------

    fn set_margins(&mut self, params: &[Option<u16>]) {
        let top = param_or(params, 0, 1).saturating_sub(1);
        // Default bottom is the last row. If the param is explicitly 0, use full screen.
        let bottom_raw = params.get(1).and_then(|v| *v);
        let bottom = match bottom_raw {
            None | Some(0) => self.dimensions.rows.saturating_sub(1),
            Some(n) => n.saturating_sub(1),
        };
        if top < bottom && bottom < self.dimensions.rows {
            self.margins = Margins { top, bottom };
            // Cursor moves to home when margins change
            if self.modes.origin {
                self.cursor = Cursor {
                    column: 0,
                    row: self.margins.top,
                };
            } else {
                self.cursor = Cursor { column: 0, row: 0 };
            }
        }
    }

    // -- alternate screen ----------------------------------------------------

    fn use_alternate_screen(&mut self, enable: bool) {
        if enable == self.using_alternate {
            return;
        }
        if enable {
            let style = Style::default();
            let alt = Grid::new(self.dimensions, style);
            self.alternate = Some(alt);
            self.using_alternate = true;
            self.cursor = Cursor { column: 0, row: 0 };
        } else {
            self.using_alternate = false;
            self.alternate = None;
            // When leaving alt screen, some terminals restore cursor
        }
    }

    // -- insert / delete -----------------------------------------------------

    fn insert_characters(&mut self, count: u16) {
        let count = count.max(1);
        let col = self.cursor.column;
        let row = self.cursor.row;
        let style = self.current_style;
        self.grid_mut().insert_cells(col, row, count, style);
    }

    fn delete_characters(&mut self, count: u16) {
        let count = count.max(1);
        let col = self.cursor.column;
        let row = self.cursor.row;
        let style = self.current_style;
        self.grid_mut().delete_cells(col, row, count, style);
    }

    fn insert_lines(&mut self, count: u16) {
        let count = count.max(1);
        let row = self.cursor.row;
        let bottom = self.margins.bottom;
        let style = self.current_style;
        self.grid_mut().insert_lines(row, bottom, count, style);
    }

    fn delete_lines(&mut self, count: u16) {
        let count = count.max(1);
        let row = self.cursor.row;
        let bottom = self.margins.bottom;
        let style = self.current_style;
        self.grid_mut().delete_lines(row, bottom, count, style);
    }

    fn erase_characters(&mut self, count: u16) {
        let count = count.max(1);
        let col = self.cursor.column;
        let row = self.cursor.row;
        let max_col = self.dimensions.columns;
        let style = self.current_style;
        let end_column = col.saturating_add(count).min(max_col);
        let grid = self.grid_mut();
        for c in col..end_column {
            grid.set_cell(c, row, Cell::blank(style));
        }
    }

    fn scroll_up_csi(&mut self, count: u16) {
        let count = count.max(1);
        let top = self.margins.top;
        let bottom = self.margins.bottom;
        let style = self.current_style;
        self.capture_scrollback(top, count);
        self.grid_mut().scroll_up(top, bottom, count, style);
    }

    fn scroll_down_csi(&mut self, count: u16) {
        let count = count.max(1);
        let top = self.margins.top;
        let bottom = self.margins.bottom;
        let style = self.current_style;
        self.grid_mut().scroll_down(top, bottom, count, style);
    }

    // -- SGR -----------------------------------------------------------------

    fn apply_sgr(&mut self, params: &[Option<u16>]) {
        if params.is_empty() || params.iter().all(Option::is_none) {
            self.current_style = Style::default();
            return;
        }

        let mut i = 0;
        while i < params.len() {
            let value = params[i].unwrap_or(0);
            match value {
                0 => self.current_style = Style::default(),
                1 => self.current_style.bold = true,
                2 => self.current_style.bold = false, // faint/dim (treat as bold off)
                3 => self.current_style.italic = true,
                4 => self.current_style.underline = true,
                5 => self.current_style.blink = true,
                7 => self.current_style.reverse = true,
                22 => self.current_style.bold = false,
                23 => self.current_style.italic = false,
                24 => self.current_style.underline = false,
                25 => self.current_style.blink = false,
                27 => self.current_style.reverse = false,
                30..=37 => {
                    self.current_style.foreground = Some(Color::Basic((value - 30) as u8));
                }
                38 => {
                    // Extended foreground
                    if i + 2 < params.len() {
                        let sub = params[i + 1].unwrap_or(0);
                        match sub {
                            5 => {
                                // 256-color
                                if i + 2 < params.len() {
                                    self.current_style.foreground =
                                        Some(Color::Indexed(params[i + 2].unwrap_or(0) as u8));
                                    i += 2;
                                }
                            }
                            2 => {
                                // True color
                                if i + 4 < params.len() {
                                    self.current_style.foreground = Some(Color::Rgb(
                                        params[i + 2].unwrap_or(0) as u8,
                                        params[i + 3].unwrap_or(0) as u8,
                                        params[i + 4].unwrap_or(0) as u8,
                                    ));
                                    i += 4;
                                }
                            }
                            _ => {
                                i += 1;
                            }
                        }
                    }
                }
                39 => self.current_style.foreground = None,
                40..=47 => {
                    self.current_style.background = Some(Color::Basic((value - 40) as u8));
                }
                48 => {
                    // Extended background
                    if i + 2 < params.len() {
                        let sub = params[i + 1].unwrap_or(0);
                        match sub {
                            5 => {
                                if i + 2 < params.len() {
                                    self.current_style.background =
                                        Some(Color::Indexed(params[i + 2].unwrap_or(0) as u8));
                                    i += 2;
                                }
                            }
                            2 => {
                                if i + 4 < params.len() {
                                    self.current_style.background = Some(Color::Rgb(
                                        params[i + 2].unwrap_or(0) as u8,
                                        params[i + 3].unwrap_or(0) as u8,
                                        params[i + 4].unwrap_or(0) as u8,
                                    ));
                                    i += 4;
                                }
                            }
                            _ => {
                                i += 1;
                            }
                        }
                    }
                }
                49 => self.current_style.background = None,
                90..=97 => {
                    self.current_style.foreground = Some(Color::Basic((value - 90 + 8) as u8));
                }
                100..=107 => {
                    self.current_style.background = Some(Color::Basic((value - 100 + 8) as u8));
                }
                _ => {}
            }
            i += 1;
        }
    }

    // -- OSC -----------------------------------------------------------------

    fn finish_osc(&mut self, bytes: &[u8]) {
        // OSC Ps ; Pt ST
        // Find the first semicolon to split number from text
        if let Some(semi) = bytes.iter().position(|&b| b == b';') {
            let number_str = &bytes[..semi];
            let text = &bytes[semi + 1..];
            // Parse the OSC number
            if let Ok(number) = std::str::from_utf8(number_str)
                .unwrap_or("0")
                .parse::<u16>()
            {
                match number {
                    0 | 2 => {
                        // Set window/tab title
                        self.title = Some(text.to_vec());
                    }
                    1 => {
                        // Set icon name – store alongside title
                        self.title = Some(text.to_vec());
                    }
                    _ => {}
                }
            }
        }
    }

    // -- character output ----------------------------------------------------

    fn put_byte(&mut self, byte: u8) {
        let insert = self.modes.insert;
        let col = self.cursor.column;
        let row = self.cursor.row;
        let auto_wrap = self.modes.auto_wrap;
        let max_col = self.dimensions.columns;
        let style = self.current_style;

        let grid = self.grid_mut();
        if insert {
            grid.insert_cells(col, row, 1, style);
        }
        grid.set_cell(
            col,
            row,
            Cell {
                bytes: vec![byte],
                style,
            },
        );
        if col + 1 < max_col {
            self.cursor.column = col + 1;
        } else if auto_wrap {
            self.cursor.column = 0;
            self.line_feed();
        }
    }

    fn horizontal_tab(&mut self) {
        let next = ((self.cursor.column / 8) + 1) * 8;
        self.cursor.column = next.min(self.dimensions.columns - 1);
    }

    fn line_feed(&mut self) {
        let bottom = self.margins.bottom;
        if self.cursor.row < bottom {
            self.cursor.row += 1;
        } else {
            let top = self.margins.top;
            let style = self.current_style;
            self.capture_scrollback(top, 1);
            self.grid_mut().scroll_up(top, bottom, 1, style);
        }
    }

    /// Capture rows that will scroll off the top into the scrollback buffer.
    fn capture_scrollback(&mut self, top: u16, count: u16) {
        if self.using_alternate {
            return;
        }
        if self.scrollback_max == 0 {
            return;
        }
        let cols = self.dimensions.columns;
        let end_row = top.saturating_add(count).min(self.dimensions.rows);
        for row in top..end_row {
            let mut line = Vec::with_capacity(usize::from(cols));
            for col in 0..cols {
                let cell = self
                    .grid()
                    .cell(col, row)
                    .cloned()
                    .unwrap_or_else(|| Cell::blank(Style::default()));
                line.push(cell);
            }
            self.push_scrollback_line(line);
        }
    }

    fn push_scrollback_line(&mut self, line: Vec<Cell>) {
        while self.scrollback.len() >= self.scrollback_max as usize {
            self.scrollback.remove(0);
        }
        self.scrollback.push(line);
    }

    // -- erase ---------------------------------------------------------------

    fn erase_display(&mut self, mode: u16) {
        let col = self.cursor.column;
        let row = self.cursor.row;
        let style = self.current_style;
        let area = self.grid().area();
        let cursor_pos = self.grid().index(col, row);
        let grid = self.grid_mut();
        match mode {
            0 => grid.clear_range(cursor_pos, area, style),
            1 => grid.clear_range(0, cursor_pos + 1, style),
            2 | 3 => grid.clear_range(0, area, style),
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let col = self.cursor.column;
        let row = self.cursor.row;
        let cols = self.dimensions.columns;
        let style = self.current_style;
        let row_start = self.grid().index(0, row);
        let cursor_idx = self.grid().index(col, row);
        let row_end = row_start + usize::from(cols);
        let grid = self.grid_mut();
        match mode {
            0 => grid.clear_range(cursor_idx, row_end, style),
            1 => grid.clear_range(row_start, cursor_idx + 1, style),
            2 => grid.clear_range(row_start, row_end, style),
            _ => {}
        }
    }

    // -- helpers -------------------------------------------------------------

    fn move_vertical(&mut self, offset: i32) {
        let top = if self.modes.origin {
            self.margins.top
        } else {
            0
        };
        let bottom = if self.modes.origin {
            self.margins.bottom
        } else {
            self.dimensions.rows - 1
        };
        let row = move_clamped(self.cursor.row, offset, top, bottom);
        self.cursor.row = row;
    }

    fn move_horizontal(&mut self, offset: i32) {
        let column = move_clamped(self.cursor.column, offset, 0, self.dimensions.columns - 1);
        self.cursor.column = column;
    }

    fn clamp_cursor(&self, cursor: Cursor) -> Cursor {
        Cursor {
            column: cursor.column.min(self.dimensions.columns - 1),
            row: cursor.row.min(self.dimensions.rows - 1),
        }
    }
}

// ---------------------------------------------------------------------------
// Parser state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum ParserState {
    Ground,
    Escape,
    Csi(CsiState),
    Osc(OscState),
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct CsiState {
    private: bool,
    params: Vec<Option<u16>>,
    current: Option<u16>,
    intermediates: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct OscState {
    bytes: Vec<u8>,
    escape_seen: bool,
}

impl OscState {
    fn push(&mut self, byte: u8) {
        const MAX_OSC_BYTES: usize = 4096;
        if self.bytes.len() < MAX_OSC_BYTES {
            self.bytes.push(byte);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn param_or(params: &[Option<u16>], index: usize, default: u16) -> u16 {
    params
        .get(index)
        .and_then(|value| *value)
        .unwrap_or(default)
}

fn move_clamped(value: u16, offset: i32, min: u16, max: u16) -> u16 {
    if offset.is_negative() {
        let abs = offset.unsigned_abs() as u16;
        value.saturating_sub(abs).max(min)
    } else {
        let abs = offset as u16;
        value.saturating_add(abs).min(max)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

fn write_uint(buf: &mut Vec<u8>, mut n: u64) {
    if n == 0 {
        buf.push(b'0');
        return;
    }
    let mut digits = [0u8; 20];
    let mut pos = 0;
    while n > 0 {
        digits[pos] = (n % 10) as u8 + b'0';
        pos += 1;
        n /= 10;
    }
    for i in (0..pos).rev() {
        buf.push(digits[i]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(columns: u16, rows: u16) -> TerminalState {
        TerminalState::new(Dimensions::new(columns, rows))
    }

    // -- Basic ---------------------------------------------------------------

    #[test]
    fn writes_printable_bytes_and_wraps() {
        let mut terminal = state(4, 2);
        terminal.apply(b"abcdef");
        assert_eq!(terminal.line_bytes(0), Some(b"abcd".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b"ef".to_vec()));
        assert_eq!(terminal.cursor, Cursor { column: 2, row: 1 });
    }

    #[test]
    fn carriage_return_line_feed_backspace_and_tab_update_cursor() {
        let mut terminal = state(10, 3);
        terminal.apply(b"abc\rZ\n12\x083\tX");
        assert_eq!(terminal.line_bytes(0), Some(b"Zbc".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b" 13     X".to_vec()));
        assert_eq!(terminal.cursor, Cursor { column: 9, row: 1 });
    }

    #[test]
    fn cursor_movement_and_erase_sequences_update_grid() {
        let mut terminal = state(6, 3);
        terminal.apply(b"abcdef\x1b[2;2HZZ\x1b[K");
        assert_eq!(terminal.line_bytes(0), Some(b"abcdef".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b" ZZ".to_vec()));
        terminal.apply(b"\x1b[2J");
        assert_eq!(terminal.plain_text(), "\n\n");
    }

    #[test]
    fn sgr_applies_to_subsequent_cells_and_can_reset() {
        let mut terminal = state(4, 1);
        terminal.apply(b"\x1b[1;4;31mA\x1b[0mB");
        let a = terminal.cell(0, 0).expect("cell A");
        assert_eq!(a.bytes, b"A");
        assert!(a.style.bold);
        assert!(a.style.underline);
        assert_eq!(a.style.foreground, Some(Color::Basic(1)));
        let b = terminal.cell(1, 0).expect("cell B");
        assert_eq!(b.bytes, b"B");
        assert_eq!(b.style, Style::default());
    }

    #[test]
    fn fragmented_escape_sequence_matches_single_chunk_parse() {
        let input = b"ab\x1b[2;3Hcd\x1b]0;title\x07ef";
        let mut whole = state(8, 3);
        whole.apply(input);
        let mut fragmented = state(8, 3);
        for chunk in input.chunks(2) {
            fragmented.apply(chunk);
        }
        assert_eq!(fragmented, whole);
        assert_eq!(fragmented.title, Some(b"title".to_vec()));
    }

    #[test]
    fn arbitrary_bytes_do_not_move_cursor_out_of_bounds() {
        let mut terminal = state(0, 0);
        for byte in 0_u8..=255 {
            terminal.apply(&[byte]);
            assert!(terminal.cursor.column < terminal.dimensions.columns);
            assert!(terminal.cursor.row < terminal.dimensions.rows);
            assert_eq!(
                terminal.grid().area(),
                usize::from(terminal.dimensions.columns) * usize::from(terminal.dimensions.rows)
            );
        }
    }

    // -- Scrolling regions ---------------------------------------------------

    #[test]
    fn scrolling_region_scrolls_only_within_margins() {
        // Margins rows 2-4 (1-indexed) = indices 1-3
        let mut terminal = state(10, 5);
        terminal.apply(b"\x1b[2;4r");
        terminal.apply(b"\x1b[1;1HAAAA");
        terminal.apply(b"\x1b[2;1HBBBB");
        terminal.apply(b"\x1b[3;1HCCCC");
        terminal.apply(b"\x1b[4;1HDDDD");
        // Cursor at bottom of margin (row 3 index), line feed triggers scroll
        terminal.apply(b"\x1b[4;1H\n");
        // Row 0 outside margin, unchanged
        assert_eq!(terminal.line_bytes(0), Some(b"AAAA".to_vec()));
        // Row 1: BBBB scrolled out, CCCC from row 2 moved here
        assert_eq!(terminal.line_bytes(1), Some(b"CCCC".to_vec()));
        // Row 2: DDDD from row 3 moved here
        assert_eq!(terminal.line_bytes(2), Some(b"DDDD".to_vec()));
        // Row 3: blank (bottom of margin cleared)
        assert_eq!(terminal.line_bytes(3), Some(b"".to_vec()));
    }

    #[test]
    fn scrolling_region_top_line_stays_when_cursor_in_region() {
        // Margins rows 2-3 (1-indexed) = indices 1-2
        let mut terminal = state(10, 5);
        terminal.apply(b"\x1b[2;3r");
        terminal.apply(b"\x1b[1;1HAAAAA");
        terminal.apply(b"\x1b[2;1HBBBBB");
        terminal.apply(b"\x1b[3;1HCCCCC");
        terminal.apply(b"\x1b[4;1HDDDDD");
        // Cursor at bottom of margin (row 2 index), line feed triggers scroll
        terminal.apply(b"\x1b[3;1H\n");
        assert_eq!(terminal.line_bytes(0), Some(b"AAAAA".to_vec()));
        // BBBBB scrolled out, CCCCC from row 2 (but row 2 is outside bottom margin?)
        // Wait: margin rows 1-2 (indices). Row 2 IS the bottom margin.
        // BBBBB at row 1, CCCCC at row 2. After scroll: BBBBB is lost, blank at row 2.
        // But CCCCC is AT the bottom margin... Let me recalculate.
        // Actually: margin top=1, bottom=2. Cursor at row 2 (the bottom).
        // Scroll_up(1, 2, 1): removes row 1 (BBBBB), shifts row 2 up to row 1, blanks row 2.
        // Row 0: AAAAA (outside), Row 1: CCCCC, Row 2: blank, Row 3: DDDDD (outside)
        assert_eq!(terminal.line_bytes(1), Some(b"CCCCC".to_vec()));
        assert_eq!(terminal.line_bytes(2), Some(b"".to_vec()));
        assert_eq!(terminal.line_bytes(3), Some(b"DDDDD".to_vec()));
    }

    // -- Alternate screen ----------------------------------------------------

    #[test]
    fn alternate_screen_preserves_primary_content() {
        let mut terminal = state(5, 3);
        terminal.apply(b"HELLO");
        assert_eq!(terminal.line_bytes(0), Some(b"HELLO".to_vec()));

        // Enter alt screen
        terminal.apply(b"\x1b[?1049h");
        assert!(terminal.using_alternate);
        // Alt screen should be blank
        assert_eq!(terminal.line_bytes(0), Some(b"".to_vec()));

        // Write in alt screen
        terminal.apply(b"WORLD");
        assert_eq!(terminal.line_bytes(0), Some(b"WORLD".to_vec()));

        // Exit alt screen
        terminal.apply(b"\x1b[?1049l");
        assert!(!terminal.using_alternate);
        // Primary content restored
        assert_eq!(terminal.line_bytes(0), Some(b"HELLO".to_vec()));
    }

    #[test]
    fn alternate_screen_1047_h_and_l() {
        let mut terminal = state(4, 2);
        terminal.apply(b"ABCD\x1b[?1047h");
        assert!(terminal.using_alternate);
        terminal.apply(b"1234");
        terminal.apply(b"\x1b[?1047l");
        assert!(!terminal.using_alternate);
        assert_eq!(terminal.line_bytes(0), Some(b"ABCD".to_vec()));
    }

    // -- Insert / delete lines -----------------------------------------------

    #[test]
    fn insert_lines_shifts_down_within_margins() {
        let mut terminal = state(10, 5);
        terminal.apply(b"\x1b[2;4r");
        terminal.apply(b"\x1b[1;1HAAAAA");
        terminal.apply(b"\x1b[2;1HBBBBB");
        terminal.apply(b"\x1b[3;1HCCCCC");
        terminal.apply(b"\x1b[4;1HDDDDD");
        terminal.apply(b"\x1b[2;1H\x1b[L");
        assert_eq!(terminal.line_bytes(0), Some(b"AAAAA".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b"".to_vec()));
        assert_eq!(terminal.line_bytes(2), Some(b"BBBBB".to_vec()));
        assert_eq!(terminal.line_bytes(3), Some(b"CCCCC".to_vec()));
    }

    #[test]
    fn delete_lines_shifts_up_within_margins() {
        let mut terminal = state(10, 5);
        terminal.apply(b"\x1b[2;4r");
        terminal.apply(b"\x1b[1;1HAAAAA");
        terminal.apply(b"\x1b[2;1HBBBBB");
        terminal.apply(b"\x1b[3;1HCCCCC");
        terminal.apply(b"\x1b[4;1HDDDDD");
        terminal.apply(b"\x1b[2;1H\x1b[M");
        assert_eq!(terminal.line_bytes(0), Some(b"AAAAA".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b"CCCCC".to_vec()));
        assert_eq!(terminal.line_bytes(2), Some(b"DDDDD".to_vec()));
        assert_eq!(terminal.line_bytes(3), Some(b"".to_vec()));
    }

    // -- Insert / delete characters ------------------------------------------

    #[test]
    fn insert_characters_shifts_right() {
        let mut terminal = state(5, 2);
        terminal.apply(b"ABCDE");
        assert_eq!(terminal.line_bytes(0), Some(b"ABCDE".to_vec()));
        // Cursor at column 1 (0-indexed), insert 1 char
        terminal.apply(b"\x1b[1;2H");
        assert_eq!(terminal.cursor, Cursor { column: 1, row: 0 });
        terminal.apply(b"\x1b[@");
        // A _ B C D  (E falls off right edge)
        assert_eq!(terminal.cursor, Cursor { column: 1, row: 0 });
        assert_eq!(terminal.line_bytes(0), Some(b"A BCD".to_vec()));
    }

    #[test]
    fn delete_characters_shifts_left() {
        let mut terminal = state(5, 2);
        terminal.apply(b"ABCDE");
        assert_eq!(terminal.line_bytes(0), Some(b"ABCDE".to_vec()));
        // Cursor at column 1, delete 1 char
        terminal.apply(b"\x1b[1;2H\x1b[P");
        // A C D E _  (B removed, rest shifts left, blank at end)
        assert_eq!(terminal.line_bytes(0), Some(b"ACDE".to_vec()));
        assert!(terminal.grid().cell(4, 0).unwrap().is_blank());
    }

    // -- Origin mode ---------------------------------------------------------

    #[test]
    fn origin_mode_positions_relative_to_margins() {
        let mut terminal = state(10, 5);
        // CSI 2;4r = margins rows 1-3 (0-indexed)
        terminal.apply(b"\x1b[2;4r");
        terminal.apply(b"\x1b[?6h"); // enable origin mode

        // CUP 1;1 should go to margin top (row 1, index)
        terminal.apply(b"\x1b[HX");
        // X written at row 1, col 0
        assert_eq!(terminal.cursor, Cursor { column: 1, row: 1 });
        assert_eq!(terminal.line_bytes(1), Some(b"X".to_vec()));

        // CUP 3;1 should go to margin top + 2 rows = row 3 (index)
        terminal.apply(b"\x1b[3;1HY");
        assert_eq!(terminal.cursor, Cursor { column: 1, row: 3 });
        assert_eq!(terminal.line_bytes(3), Some(b"Y".to_vec()));

        // Disable origin mode
        terminal.apply(b"\x1b[?6l");
        terminal.apply(b"\x1b[HZ");
        assert_eq!(terminal.cursor, Cursor { column: 1, row: 0 });
    }

    // -- 256-color + true color ----------------------------------------------

    #[test]
    fn sgr_256_color_foreground() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[38;5;196mA");
        let cell = terminal.cell(0, 0).unwrap();
        assert_eq!(cell.style.foreground, Some(Color::Indexed(196)));
    }

    #[test]
    fn sgr_true_color_foreground() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[38;2;100;150;200mA");
        let cell = terminal.cell(0, 0).unwrap();
        assert_eq!(cell.style.foreground, Some(Color::Rgb(100, 150, 200)));
    }

    #[test]
    fn sgr_true_color_background() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[48;2;50;60;70mA");
        let cell = terminal.cell(0, 0).unwrap();
        assert_eq!(cell.style.background, Some(Color::Rgb(50, 60, 70)));
    }

    #[test]
    fn sgr_bright_colors() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[91mA");
        let cell = terminal.cell(0, 0).unwrap();
        // 91 = bright red = Basic(9) (index 8 = bright black, 9 = bright red)
        assert_eq!(cell.style.foreground, Some(Color::Basic(9)));
    }

    #[test]
    fn sgr_bright_background() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[101mA");
        let cell = terminal.cell(0, 0).unwrap();
        assert_eq!(cell.style.background, Some(Color::Basic(9)));
    }

    // -- DEC private modes ---------------------------------------------------

    #[test]
    fn application_cursor_mode() {
        let mut terminal = state(5, 2);
        assert!(!terminal.modes.application_cursor);
        terminal.apply(b"\x1b[?1h");
        assert!(terminal.modes.application_cursor);
        terminal.apply(b"\x1b[?1l");
        assert!(!terminal.modes.application_cursor);
    }

    #[test]
    fn show_hide_cursor() {
        let mut terminal = state(5, 2);
        assert!(terminal.modes.show_cursor);
        terminal.apply(b"\x1b[?25l");
        assert!(!terminal.modes.show_cursor);
        terminal.apply(b"\x1b[?25h");
        assert!(terminal.modes.show_cursor);
    }

    #[test]
    fn auto_wrap_mode() {
        let mut terminal = state(4, 2);
        assert!(terminal.modes.auto_wrap);
        terminal.apply(b"\x1b[?7l");
        assert!(!terminal.modes.auto_wrap);
        // With auto-wrap off, writing past right edge shouldn't line feed
        terminal.apply(b"ABCDE");
        assert_eq!(terminal.cursor, Cursor { column: 3, row: 0 });
    }

    // -- Cursor save/restore ------------------------------------------------

    #[test]
    fn cursor_save_restore_preserves_style() {
        let mut terminal = state(5, 3);
        terminal.apply(b"\x1b[1;31m"); // bold red
        terminal.apply(b"\x1b[s"); // save (ANSI)
        terminal.apply(b"\x1b[0mX"); // reset, write
        assert_eq!(terminal.current_style(), Style::default());
        terminal.apply(b"\x1b[u"); // restore
        assert!(terminal.current_style().bold);
        assert_eq!(terminal.current_style().foreground, Some(Color::Basic(1)));
    }

    // -- Scroll up/down CSI --------------------------------------------------

    #[test]
    fn csi_scroll_up() {
        // Use 5+ cols so writing 4 chars on last row doesn't trigger auto-wrap+scroll
        let mut terminal = state(8, 3);
        terminal.apply(b"AAAA\x1b[2;1HBBBB\x1b[3;1HCCCC");
        assert_eq!(terminal.line_bytes(0), Some(b"AAAA".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b"BBBB".to_vec()));
        assert_eq!(terminal.line_bytes(2), Some(b"CCCC".to_vec()));
        terminal.apply(b"\x1b[S");
        assert_eq!(terminal.line_bytes(0), Some(b"BBBB".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b"CCCC".to_vec()));
        assert_eq!(terminal.line_bytes(2), Some(b"".to_vec()));
    }

    #[test]
    fn csi_scroll_down() {
        let mut terminal = state(8, 3);
        terminal.apply(b"AAAA\x1b[2;1HBBBB\x1b[3;1HCCCC");
        terminal.apply(b"\x1b[T");
        assert_eq!(terminal.line_bytes(0), Some(b"".to_vec()));
        assert_eq!(terminal.line_bytes(1), Some(b"AAAA".to_vec()));
        assert_eq!(terminal.line_bytes(2), Some(b"BBBB".to_vec()));
    }

    // -- Cursor positioning variants -----------------------------------------

    #[test]
    fn cursor_next_line() {
        let mut terminal = state(5, 3);
        terminal.apply(b"\x1b[2E"); // next line x 2
        assert_eq!(terminal.cursor, Cursor { column: 0, row: 2 });
    }

    #[test]
    fn cursor_horizontal_absolute() {
        let mut terminal = state(5, 3);
        terminal.apply(b"\x1b[3G");
        assert_eq!(terminal.cursor, Cursor { column: 2, row: 0 });
    }

    #[test]
    fn cursor_line_absolute() {
        let mut terminal = state(5, 5);
        terminal.apply(b"\x1b[3d");
        assert_eq!(terminal.cursor, Cursor { column: 0, row: 2 });
    }

    // -- Erase characters ----------------------------------------------------

    #[test]
    fn erase_characters_clears_right() {
        let mut terminal = state(6, 2);
        terminal.apply(b"ABCDEF");
        // Cursor at col 1 (0-indexed), erase 3 chars
        terminal.apply(b"\x1b[1;2H\x1b[3X");
        // A _ _ _ E F
        assert_eq!(terminal.line_bytes(0), Some(b"A   EF".to_vec()));
    }

    // -- OSC title -----------------------------------------------------------

    #[test]
    fn osc_2_sets_title() {
        let mut terminal = state(5, 2);
        terminal.apply(b"\x1b]2;hello world\x07");
        assert_eq!(terminal.title, Some(b"hello world".to_vec()));
    }

    #[test]
    fn osc_0_sets_title() {
        let mut terminal = state(5, 2);
        terminal.apply(b"\x1b]0;my title\x07");
        assert_eq!(terminal.title, Some(b"my title".to_vec()));
    }

    // -- DECSC/DECRC ---------------------------------------------------------

    #[test]
    fn decsc_decirc_saves_and_restores() {
        let mut terminal = state(8, 4);
        terminal.apply(b"\x1b[3;5H\x1b[1;31m");
        terminal.apply(b"\x1b7"); // DECSC
        terminal.apply(b"\x1b[H\x1b[0mX");
        // Cursor should be at (1,0)
        assert_eq!(terminal.cursor, Cursor { column: 1, row: 0 });
        terminal.apply(b"\x1b8"); // DECRC
        // Back to (4,2) (0-indexed: row 2, col 4)
        assert_eq!(terminal.cursor, Cursor { column: 4, row: 2 });
        assert!(terminal.current_style().bold);
    }

    // -- SGR edge cases ------------------------------------------------------

    #[test]
    fn sgr_reset_with_params_all_none() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[1;31mA"); // bold red
        terminal.apply(b"\x1b[mB"); // empty/none params = reset
        let cell = terminal.cell(1, 0).unwrap();
        assert_eq!(cell.style, Style::default());
    }

    #[test]
    fn sgr_italic_blink_reverse() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[3;5;7mA");
        let cell = terminal.cell(0, 0).unwrap();
        assert!(cell.style.italic);
        assert!(cell.style.blink);
        assert!(cell.style.reverse);
    }

    #[test]
    fn sgr_negative_disable() {
        let mut terminal = state(3, 1);
        terminal.apply(b"\x1b[3;5;7m\x1b[23;25;27mB");
        let cell = terminal.cell(0, 0).unwrap();
        assert!(!cell.style.italic);
        assert!(!cell.style.blink);
        assert!(!cell.style.reverse);
    }

    // -- Resize --------------------------------------------------------------

    #[test]
    fn resize_handles_shrink_and_grow() {
        let mut terminal = state(10, 5);
        terminal.apply(b"HELLO");
        terminal.resize(Dimensions::new(3, 3));
        // Cursor clamped
        assert!(terminal.cursor.column < 3);
        assert!(terminal.cursor.row < 3);
        assert_eq!(terminal.dimensions, Dimensions::new(3, 3));
    }

    // -- Property: no panic on arbitrary bytes -------------------------------

    #[test]
    fn exhaustive_byte_fuzz_no_panic() {
        let mut terminal = state(80, 24);
        // Apply a long random-looking sequence
        for byte in 0u8..=255 {
            for _ in 0..5 {
                terminal.apply(&[byte]);
            }
        }
        // Should not panic
        assert!(terminal.cursor.column < terminal.dimensions.columns);
        assert!(terminal.cursor.row < terminal.dimensions.rows);
    }

    #[test]
    fn malformed_csi_does_not_panic() {
        let mut terminal = state(80, 24);
        // Random bytes that look like partial CSI
        terminal.apply(b"\x1b[99999999999999999999999A");
        terminal.apply(b"\x1b[;;;;;;;;;;m");
        terminal.apply(b"\x1b[?999999h");
        terminal.apply(b"\x1b[38:5:196m"); // colon-delimited (not supported, should be safe)
        // No panics
        assert!(terminal.cursor.row < terminal.dimensions.rows);
    }
}
