use ratatui::style::{Color, Modifier, Style};

pub struct Theme {
    pub name: String,
    // Editor
    pub editor_bg: Color,
    pub editor_fg: Color,
    pub editor_cursor: Color,
    pub editor_selection: Color,
    pub editor_line_number: Color,
    pub editor_line_number_active: Color,
    pub editor_gutter_bg: Color,
    // Sidebar
    pub sidebar_bg: Color,
    pub sidebar_fg: Color,
    pub sidebar_active_bg: Color,
    pub sidebar_active_fg: Color,
    // Tabs
    pub tab_bg: Color,
    pub tab_fg: Color,
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_dirty: Color,
    // Status bar
    pub statusbar_bg: Color,
    pub statusbar_fg: Color,
    // Diff
    pub diff_add_bg: Color,
    pub diff_add_fg: Color,
    pub diff_del_bg: Color,
    pub diff_del_fg: Color,
    pub diff_hunk_bg: Color,
    pub diff_hunk_fg: Color,
    // Syntax
    pub syntax_keyword: Color,
    pub syntax_string: Color,
    pub syntax_comment: Color,
    pub syntax_function: Color,
    pub syntax_type: Color,
    pub syntax_number: Color,
    pub syntax_operator: Color,
    pub syntax_variable: Color,
    pub syntax_constant: Color,
    // Picker
    pub picker_bg: Color,
    pub picker_fg: Color,
    pub picker_match: Color,
    pub picker_selected: Color,
    // Command bar
    pub command_bar_bg: Color,
    pub command_bar_fg: Color,
    pub command_bar_prompt: Color,
    pub command_bar_placeholder: Color,
    pub command_bar_info_bg: Color,
    pub command_bar_info_fg: Color,
    pub command_bar_info_accent: Color,
    // Borders
    pub border: Color,
}

impl Theme {
    /// VS Code Dark+ inspired theme with carefully chosen truecolor values
    pub fn dark_plus() -> Self {
        Self {
            name: "Dark+".to_string(),
            // Editor — VS Code Dark+ background #1E1E1E
            editor_bg: Color::Rgb(30, 30, 30),
            editor_fg: Color::Rgb(212, 212, 212),
            editor_cursor: Color::Rgb(174, 175, 173),
            editor_selection: Color::Rgb(38, 79, 120),
            editor_line_number: Color::Rgb(90, 90, 90),
            editor_line_number_active: Color::Rgb(200, 200, 200),
            editor_gutter_bg: Color::Rgb(30, 30, 30),
            // Sidebar — slightly darker
            sidebar_bg: Color::Rgb(37, 37, 38),
            sidebar_fg: Color::Rgb(204, 204, 204),
            sidebar_active_bg: Color::Rgb(4, 57, 94),
            sidebar_active_fg: Color::Rgb(255, 255, 255),
            // Tabs
            tab_bg: Color::Rgb(45, 45, 45),
            tab_fg: Color::Rgb(150, 150, 150),
            tab_active_bg: Color::Rgb(30, 30, 30),
            tab_active_fg: Color::Rgb(255, 255, 255),
            tab_dirty: Color::Rgb(204, 176, 96),
            // Status bar — VS Code blue
            statusbar_bg: Color::Rgb(0, 122, 204),
            statusbar_fg: Color::Rgb(255, 255, 255),
            // Diff
            diff_add_bg: Color::Rgb(35, 61, 41),
            diff_add_fg: Color::Rgb(180, 235, 180),
            diff_del_bg: Color::Rgb(72, 30, 30),
            diff_del_fg: Color::Rgb(235, 180, 180),
            diff_hunk_bg: Color::Rgb(30, 50, 75),
            diff_hunk_fg: Color::Rgb(140, 180, 220),
            // Syntax — VS Code Dark+ token colors
            syntax_keyword: Color::Rgb(86, 156, 214),    // blue — if, fn, let, pub
            syntax_string: Color::Rgb(206, 145, 120),     // orange — "strings"
            syntax_comment: Color::Rgb(106, 153, 85),     // green — // comments
            syntax_function: Color::Rgb(220, 220, 170),   // yellow — function names
            syntax_type: Color::Rgb(78, 201, 176),        // teal — type names
            syntax_number: Color::Rgb(181, 206, 168),     // light green — numbers
            syntax_operator: Color::Rgb(212, 212, 212),   // light grey — operators
            syntax_variable: Color::Rgb(156, 220, 254),   // light blue — variables
            syntax_constant: Color::Rgb(100, 150, 224),   // medium blue — constants
            // Picker
            picker_bg: Color::Rgb(37, 37, 38),
            picker_fg: Color::Rgb(204, 204, 204),
            picker_match: Color::Rgb(18, 133, 201),
            picker_selected: Color::Rgb(4, 57, 94),
            // Command bar
            command_bar_bg: Color::Rgb(24, 24, 24),
            command_bar_fg: Color::Rgb(204, 204, 204),
            command_bar_prompt: Color::Rgb(13, 147, 115),
            command_bar_placeholder: Color::Rgb(90, 90, 90),
            command_bar_info_bg: Color::Rgb(37, 37, 38),
            command_bar_info_fg: Color::Rgb(130, 130, 130),
            command_bar_info_accent: Color::Rgb(75, 140, 200),
            // Borders
            border: Color::Rgb(60, 60, 60),
        }
    }

    pub fn style_for_token(&self, token_type: &str) -> Style {
        let color = match token_type {
            "keyword" | "keyword.control" | "keyword.function" | "keyword.operator"
            | "keyword.return" | "keyword.storage" | "keyword.type" | "keyword.modifier"
            | "keyword.import" | "keyword.conditional" | "keyword.repeat" | "keyword.exception" => {
                self.syntax_keyword
            }
            "string" | "string.special" => self.syntax_string,
            "comment" | "comment.line" | "comment.block" | "comment.documentation" => {
                self.syntax_comment
            }
            "function" | "function.call" | "function.method" | "function.builtin"
            | "method" | "method.call" => self.syntax_function,
            "type" | "type.builtin" | "type.definition" | "constructor" | "class" => {
                self.syntax_type
            }
            "number" | "float" | "integer" => self.syntax_number,
            "operator" | "punctuation" | "punctuation.bracket" | "punctuation.delimiter"
            | "punctuation.special" => self.syntax_operator,
            "variable" | "variable.parameter" | "variable.builtin" | "property"
            | "field" | "parameter" => self.syntax_variable,
            "constant" | "constant.builtin" | "boolean" | "attribute" | "label" => {
                self.syntax_constant
            }
            _ => self.editor_fg,
        };

        let mut style = Style::default().fg(color);
        if token_type.starts_with("keyword") {
            // No bold to match VS Code
        }
        if token_type.starts_with("comment") {
            style = style.add_modifier(Modifier::ITALIC);
        }
        style
    }
}
