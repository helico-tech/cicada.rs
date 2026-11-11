//! Drawing: header (info, key hints, logo), prompt, the screen, breadcrumbs and toast, and the dialogs over them.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Flex, Layout, Margin, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Clear, Paragraph, Row, Table, TableState, Wrap};

use super::app::{Action, App, Hit, Item, Kind, Mode, Target, label};
use super::highlight;
use super::theme::{self, ACCENT, KEY, LABEL, LOGO, LOGO_LINES, Tone};
use crate::output::{age, wire};

const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
const HEADER_HEIGHT: u16 = 6;

/// Draws the whole screen; records where the table is for the mouse.
pub fn draw(frame: &mut Frame, app: &mut App) {
    let area = frame.area();
    let header = if area.height >= 20 { HEADER_HEIGHT } else { 0 };
    let prompt = if matches!(app.mode, Mode::Command(_) | Mode::Filter) { 3 } else { 0 };
    let [header_area, prompt_area, body, footer] = Layout::vertical([
        Constraint::Length(header),
        Constraint::Length(prompt),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(area);
    if header > 0 {
        draw_header(frame, app, header_area);
    }
    if prompt > 0 {
        draw_prompt(frame, app, prompt_area);
    }
    app.hit = None;
    draw_body(frame, app, body);
    draw_footer(frame, app, footer);
    match &app.mode {
        Mode::Confirm { action, yes } => draw_confirm(frame, action, *yes),
        Mode::Help => draw_help(frame),
        _ => {}
    }
}

// ---- header ----

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let logo_width = if area.width >= 110 { LOGO_LINES[0].chars().count() as u16 + 2 } else { 0 };
    let [info, hints, logo] =
        Layout::horizontal([Constraint::Length(50), Constraint::Min(0), Constraint::Length(logo_width)]).areas(area);
    frame.render_widget(Paragraph::new(info_lines(app)), info);
    frame.render_widget(Paragraph::new(hint_lines(&hints_for(app), area.height as usize)), hints);
    if logo_width > 0 {
        let mut lines: Vec<Line> = LOGO_LINES.iter().map(|l| Line::from(l.fg(LOGO).bold())).collect();
        lines.push(Line::from("durable actors".dim()).alignment(Alignment::Right));
        frame.render_widget(Paragraph::new(lines), logo);
    }
}

fn info_lines(app: &App) -> Vec<Line<'static>> {
    let row = |label: &'static str, value: Vec<Span<'static>>| {
        let mut spans = vec![Span::styled(format!("{label:<9}"), Style::new().fg(LABEL))];
        spans.extend(value);
        Line::from(spans)
    };
    let (nodes, load, queue) = match &app.summary {
        None => (vec!["…".dim()], vec!["…".dim()], vec!["…".dim()]),
        Some(Err(error)) => {
            (vec![Span::styled(format!("● unreachable: {error}"), Tone::Bad.style())], vec!["-".dim()], vec!["-".dim()])
        }
        Some(Ok(summary)) => {
            let mut nodes = Vec::new();
            for health in ["alive", "suspect", "dead"] {
                let n = summary.nodes.iter().filter(|node| wire(&node.status) == health).count();
                if n > 0 {
                    nodes.push(Span::styled(format!("{n} ● {health}  "), theme::status(health).style()));
                }
            }
            if nodes.is_empty() {
                nodes.push(Span::styled("none", Tone::Warn.style()));
            }
            let turns: usize = summary.nodes.iter().map(|n| n.running_turns).sum();
            let stats = &summary.stats;
            let load = vec![
                Span::styled(stats.actors.to_string(), theme::bold()),
                " actors · ".dim(),
                Span::styled(stats.pending.to_string(), theme::bold()),
                " pending · ".dim(),
                Span::styled(turns.to_string(), if turns > 0 { Tone::Info.style().bold() } else { theme::bold() }),
                " turns".dim(),
            ];
            let queue = match stats.oldest_due {
                Some(t) => {
                    vec![Span::styled(format!("oldest due {} ago", age(t, chrono::Utc::now())), Tone::Warn.style())]
                }
                None => vec![Span::styled("nothing due", Tone::Good.style())],
            };
            (nodes, load, queue)
        }
    };
    vec![
        row("Context:", vec![Span::styled(app.context.clone().unwrap_or_else(|| "-".into()), theme::bold())]),
        row("Server:", vec![Span::styled(app.server.clone(), theme::bold())]),
        row("Nodes:", nodes),
        row("Load:", load),
        row("Queue:", queue),
        row("Version:", vec![Span::from(format!("cicada {}", env!("CARGO_PKG_VERSION"))).dim()]),
    ]
}

/// The key hints of the current screen.
fn hints_for(app: &App) -> Vec<(&'static str, &'static str)> {
    let screen = app.screen();
    if screen.describe.is_some() {
        return vec![
            ("esc", "back"),
            ("t", "yaml/json"),
            ("w", "wrap"),
            ("j/k", "scroll"),
            ("g/G", "top/end"),
            (":", "command"),
            ("?", "help"),
        ];
    }
    let mut hints =
        vec![(":", "command"), ("/", "filter"), ("?", "help"), ("esc", "back"), ("enter", "open"), ("d", "describe")];
    let specific: &[(&str, &str)] = match &screen.target {
        Target::List { kind: Kind::Actors, .. } => &[
            ("m", "mailbox"),
            ("p", "pause"),
            ("u", "resume"),
            ("x", "purge"),
            ("ctrl-k", "stop turn"),
            ("ctrl-d", "delete"),
        ],
        Target::List { kind: Kind::Messages, .. } => &[("c", "cancel"), ("n", "deliver now")],
        Target::Mailbox(_) => &[("c", "cancel"), ("n", "deliver now"), ("p", "pause"), ("u", "resume"), ("x", "purge")],
        Target::List { kind: Kind::Outcomes, .. } => &[("r", "retry")],
        Target::List { kind: Kind::Turns, .. } | Target::Node(_) => &[("ctrl-k", "stop turn")],
        Target::List { kind: Kind::Types, .. } | Target::Type(_) => &[("ctrl-d", "remove")],
        Target::Actor(_) => &[
            ("tab", "switch tab"),
            ("P", "parent"),
            ("m", "mailbox"),
            ("c", "cancel"),
            ("r", "retry"),
            ("n", "deliver now"),
            ("p", "pause"),
            ("u", "resume"),
            ("x", "purge"),
            ("ctrl-k", "stop turn"),
            ("ctrl-d", "delete"),
        ],
        Target::Message(_) => &[("a", "actor"), ("b", "cause"), ("c", "cancel"), ("r", "retry"), ("n", "deliver now")],
        Target::List { kind: Kind::Nodes, .. } | Target::Describe(_) => &[],
    };
    hints.extend(specific);
    hints.extend([("< >", "sort"), ("w", "wide")]);
    hints
}

/// Hints in columns of `height` rows: `<key> label`.
fn hint_lines(hints: &[(&'static str, &'static str)], height: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Vec<Span<'static>>> = vec![Vec::new(); height];
    for column in hints.chunks(height.max(1)) {
        let keys = column.iter().map(|(k, _)| k.len()).max().unwrap_or(0) + 2;
        let labels = column.iter().map(|(_, l)| l.len()).max().unwrap_or(0) + 2;
        for (row, line) in lines.iter_mut().enumerate() {
            match column.get(row) {
                Some((key, label)) => {
                    // `<` and `>` are keys themselves; brackets around them would read as `<< >>`.
                    let key = if key.contains('<') { key.to_string() } else { format!("<{key}>") };
                    line.push(Span::styled(format!("{key:<keys$}"), Style::new().fg(KEY).bold()));
                    line.push(Span::styled(format!(" {label:<labels$}"), Style::new().fg(Color::Gray)));
                }
                None => line.push(Span::raw(" ".repeat(keys + labels + 1))),
            }
        }
    }
    lines.into_iter().map(Line::from).collect()
}

// ---- prompt ----

fn draw_prompt(frame: &mut Frame, app: &App, area: Rect) {
    let (title, sigil, text, ghost) = match &app.mode {
        Mode::Command(text) => {
            let ghost = App::completion(text).and_then(|c| c.get(text.len()..).map(str::to_owned)).unwrap_or_default();
            (" command ", ":", text.clone(), ghost)
        }
        _ => (" filter ", "/", app.screen().grid().map(|g| g.filter.clone()).unwrap_or_default(), String::new()),
    };
    let line = Line::from(vec![
        Span::styled(format!(" {sigil} "), Style::new().fg(ACCENT).bold()),
        Span::styled(text, theme::bold()),
        Span::styled(ghost, Style::new().fg(Color::DarkGray)),
        Span::styled("▏", Style::new().fg(ACCENT)),
    ]);
    let hint = if sigil == ":" {
        " tab completes · enter runs · esc cancels "
    } else {
        " live · enter keeps · esc clears "
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(title.bold())
        .title_bottom(Line::from(hint.dim()).right_aligned());
    frame.render_widget(Paragraph::new(line).block(block), area);
}

// ---- body ----

fn draw_body(frame: &mut Frame, app: &mut App, area: Rect) {
    let screen = app.screen();
    if screen.describe.is_some() {
        return draw_describe(frame, app, area);
    }
    if !screen.loaded()
        && let Some(error) = &screen.error
    {
        let block =
            title_block(Line::from(format!(" {} ", screen.target.title()).bold())).border_style(Tone::Bad.style());
        let text = vec![
            Line::from(""),
            Line::from(Span::styled(format!("✗ {error}"), Tone::Bad.style())).centered(),
            Line::from("esc goes back · ctrl-r tries again".dim()).centered(),
        ];
        frame.render_widget(Paragraph::new(text).wrap(Wrap { trim: true }).block(block), area);
        return;
    }
    if screen.overview.is_some() {
        draw_overview(frame, app, area);
    } else {
        draw_grid(frame, app, area);
    }
}

fn title_block(title: Line<'_>) -> Block<'_> {
    Block::bordered().border_style(Style::new().fg(ACCENT)).title(title.centered())
}

fn spinner(app: &App) -> &'static str {
    if app.loading() { SPINNER[(app.tick % SPINNER.len() as u64) as usize] } else { " " }
}

fn draw_grid(frame: &mut Frame, app: &mut App, area: Rect) {
    let (wide, spin) = (app.wide, spinner(app));
    let screen = app.stack.last_mut().expect("the stack is never empty");
    let mut title = vec![Span::raw(" ")];
    if screen.tabs.len() > 1 || screen.tabs.first().is_some_and(|(name, _)| !name.is_empty()) {
        for (i, (name, grid)) in screen.tabs.iter().enumerate() {
            if i > 0 {
                title.push(" │ ".fg(Color::DarkGray));
            }
            let text = format!(" {name}[{}] ", grid.shown().len());
            title.push(if i == screen.tab {
                Span::styled(text, Style::new().fg(Color::Black).bg(ACCENT).bold())
            } else {
                Span::styled(text, Style::new().fg(ACCENT))
            });
        }
    } else if let Some((_, grid)) = screen.tabs.first() {
        title.push(Span::styled(screen.target.title(), Style::new().fg(ACCENT).bold()));
        title.push(Span::styled(format!("[{}]", grid.shown().len()), theme::bold()));
    }
    let error = screen.error.clone();
    let Some(grid) = screen.grid_mut() else { return };
    if !grid.filter.is_empty() {
        title.push(Span::styled(format!(" </{}>", grid.filter), Style::new().fg(LABEL).bold()));
    }
    title.push(Span::styled(format!(" {spin} "), Style::new().fg(ACCENT)));
    let mut block = title_block(Line::from(title));
    if let Some(error) = error {
        block = block.title_bottom(Span::styled(format!(" ✗ refresh failed: {error} "), Tone::Bad.style()));
    }
    let inner = block.inner(area);

    let headers = if wide { &grid.data.wide_headers } else { &grid.data.headers };
    let shown = grid.shown();
    if shown.is_empty() {
        let text = match () {
            _ if !grid.loaded => "loading…".to_owned(),
            _ if !grid.filter.is_empty() => format!("no rows match /{}", grid.filter),
            _ => "nothing here".to_owned(),
        };
        frame.render_widget(block, area);
        let [middle] = Layout::vertical([Constraint::Length(1)]).flex(Flex::Center).areas(inner);
        frame.render_widget(Paragraph::new(text.dim()).centered(), middle);
        return;
    }
    let status_column = headers.iter().position(|h| theme::is_status_column(h));
    let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count() + 1).collect();
    let rows: Vec<Row> = shown
        .iter()
        .map(|&i| {
            let cells = if wide { &grid.data.rows[i].wide } else { &grid.data.rows[i].cells };
            let row_tone = theme::row(status_column.and_then(|c| cells.get(c)).map(String::as_str));
            Row::new(cells.iter().zip(headers.iter()).enumerate().map(|(c, (value, header))| {
                let tone = match theme::cell(header, value) {
                    Tone::Normal => row_tone,
                    tone => tone,
                };
                let text = if theme::is_status_column(header) { format!("● {value}") } else { value.clone() };
                if let Some(width) = widths.get_mut(c) {
                    *width = (*width).max(text.chars().count()).min(50);
                }
                Cell::from(text).style(tone.style())
            }))
        })
        .collect();
    let last = widths.len().saturating_sub(1);
    let constraints: Vec<Constraint> = widths
        .iter()
        .enumerate()
        .map(|(i, w)| if i == last { Constraint::Fill(1) } else { Constraint::Length(*w as u16) })
        .collect();
    let header_cells = headers.iter().enumerate().map(|(i, h)| match grid.sort {
        Some((c, descending)) if c == i => format!("{h}{}", if descending { "↓" } else { "↑" }),
        _ => h.to_string(),
    });
    let table = Table::new(rows, constraints.clone())
        .header(Row::new(header_cells).style(Style::new().fg(ACCENT).bold()))
        .column_spacing(2)
        .row_highlight_style(Style::new().fg(Color::Black).bg(ACCENT).add_modifier(Modifier::BOLD))
        .block(block);
    let mut state = TableState::new().with_offset(grid.offset).with_selected(Some(grid.selected));
    frame.render_stateful_widget(table, area, &mut state);
    grid.offset = state.offset();
    let columns = Layout::horizontal(constraints).spacing(2).split(inner).iter().map(|r| (r.x, r.right())).collect();
    app.hit = Some(Hit { area: inner, columns });
}

fn draw_overview(frame: &mut Frame, app: &mut App, area: Rect) {
    let screen = app.screen();
    let overview = screen.overview.clone().expect("an overview");
    let top_height = (overview.facts.len() as u16 + 2).max(9).min(area.height / 2);
    let [top, bottom] = Layout::vertical([Constraint::Length(top_height), Constraint::Min(0)]).areas(area);
    let facts_share = if overview.panes.is_empty() { 100 } else { 50 };
    let [facts_area, pane_area] =
        Layout::horizontal([Constraint::Percentage(facts_share), Constraint::Fill(1)]).areas(top);

    let label_width = overview.facts.iter().map(|f| f.label.len()).max().unwrap_or(0) + 2;
    let facts: Vec<Line> = overview
        .facts
        .iter()
        .map(|f| {
            let value = if f.tone == Tone::Normal { theme::bold() } else { f.tone.style() };
            Line::from(vec![
                Span::styled(format!("{:<label_width$}", f.label), Style::new().fg(LABEL)),
                Span::styled(f.value.clone(), value),
            ])
        })
        .collect();
    let title = Line::from(vec![
        Span::styled(format!(" {} ", label(&overview.subject)), Style::new().fg(ACCENT).bold()),
        Span::styled(format!("{} ", spinner(app)), Style::new().fg(ACCENT)),
    ]);
    frame.render_widget(Paragraph::new(facts).wrap(Wrap { trim: false }).block(title_block(title)), facts_area);

    let pane = match screen.grid().and_then(|g| g.selected_item()) {
        Some(Item::Schema(name)) if overview.pane_follows_selection => overview.panes.iter().find(|(n, _)| n == name),
        _ => overview.panes.first(),
    };
    if let Some((name, value)) = pane {
        let block = title_block(Line::from(Span::styled(format!(" {name} "), Style::new().fg(ACCENT).bold())));
        frame.render_widget(Paragraph::new(highlight::json(value)).block(block), pane_area);
    }
    draw_grid(frame, app, bottom);
}

fn draw_describe(frame: &mut Frame, app: &mut App, area: Rect) {
    let spin = spinner(app);
    let screen = app.stack.last_mut().expect("the stack is never empty");
    let title = screen.target.title();
    let Some(describe) = screen.describe.as_mut() else { return };
    let lines = if describe.yaml { highlight::yaml(&describe.value) } else { highlight::json(&describe.value) };
    let visible = area.height.saturating_sub(2);
    let total = lines.len() as u16;
    describe.scroll = describe.scroll.min(total.saturating_sub(visible));
    let format = if describe.yaml { "yaml" } else { "json" };
    let title = Line::from(vec![
        Span::styled(format!(" describe({title}) "), Style::new().fg(ACCENT).bold()),
        Span::styled(format!("[{format}] "), Style::new().fg(LABEL)),
        Span::styled(format!("{spin} "), Style::new().fg(ACCENT)),
    ]);
    let position = format!(" {}-{} of {total} ", describe.scroll + 1, (describe.scroll + visible).min(total));
    let block = title_block(title).title_bottom(Line::from(position.dim()).right_aligned());
    let mut paragraph = Paragraph::new(lines).block(block).scroll((describe.scroll, 0));
    if describe.wrap {
        paragraph = paragraph.wrap(Wrap { trim: false });
    }
    frame.render_widget(paragraph, area);
}

// ---- footer ----

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let last = app.stack.len() - 1;
    let mut crumbs = Vec::new();
    for (i, screen) in app.stack.iter().enumerate() {
        let style = if i == last {
            Style::new().fg(Color::Black).bg(LABEL).bold()
        } else {
            Style::new().fg(Color::Black).bg(ACCENT)
        };
        crumbs.push(Span::styled(format!(" <{}> ", screen.target.crumb()), style));
        crumbs.push(Span::raw(" "));
    }
    let toast = app.toast.as_ref().map(|t| {
        let (sigil, tone) = if t.error { ("✗", Tone::Bad) } else { ("✓", Tone::Good) };
        Span::styled(format!("{sigil} {} ", t.text), tone.style().bold())
    });
    let toast_width = toast.as_ref().map_or(0, |t| t.width() as u16).min(area.width * 2 / 3);
    let [left, right] = Layout::horizontal([Constraint::Min(0), Constraint::Length(toast_width)]).areas(area);
    frame.render_widget(Paragraph::new(Line::from(crumbs)), left);
    if let Some(toast) = toast {
        frame.render_widget(Paragraph::new(Line::from(toast)).right_aligned(), right);
    }
}

// ---- dialogs ----

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let [area] = Layout::vertical([Constraint::Length(height.min(area.height))]).flex(Flex::Center).areas(area);
    let [area] = Layout::horizontal([Constraint::Length(width.min(area.width))]).flex(Flex::Center).areas(area);
    area
}

fn draw_confirm(frame: &mut Frame, action: &Action, yes: bool) {
    let area = centered(frame.area(), 64, 8);
    frame.render_widget(Clear, area);
    let (color, title) = if action.dangerous() { (Color::Red, " ⚠ Danger ") } else { (Color::Yellow, " Confirm ") };
    let button = |text: &'static str, focused: bool| {
        if focused {
            Span::styled(text, Style::new().fg(Color::Black).bg(color).bold())
        } else {
            Span::styled(text, Style::new().fg(color))
        }
    };
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(action.question(), theme::bold())).centered(),
        Line::from(""),
        Line::from(vec![button("  Yes  ", yes), Span::raw("     "), button("  No  ", !yes)]).centered(),
    ];
    let block = Block::bordered()
        .border_type(BorderType::Double)
        .border_style(Style::new().fg(color))
        .title(Line::from(title.bold()).centered())
        .title_bottom(Line::from(" y yes · n/esc no · ←→ choose · enter ".dim()).centered());
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }).block(block), area);
}

type Section = (&'static str, &'static [(&'static str, &'static str)]);

const HELP: [Section; 4] = [
    (
        "RESOURCES",
        &[
            (":actors [prefix]", "actors"),
            (":messages [prefix]", "pending messages"),
            (":outcomes [status] [prefix]", "finished messages"),
            (":turns [prefix]", "running turns"),
            (":nodes", "nodes"),
            (":types", "registered types"),
            (":actor ADDRESS", "one actor"),
            (":mailbox ADDRESS", "one mailbox"),
            (":message ID", "one message"),
            (":node ID", "one node"),
            (":type NAME", "one type"),
            (":q  ctrl-c", "quit"),
        ],
    ),
    (
        "NAVIGATION",
        &[
            ("j k ↑ ↓", "move"),
            ("g G pgup pgdn", "first, last, page"),
            ("enter", "open"),
            ("esc  q", "clear filter, back"),
            ("tab", "next tab"),
            ("/", "filter rows as you type"),
            ("< >  s", "sort column, order"),
            ("w", "wide columns"),
            ("ctrl-r", "refresh now"),
            ("mouse", "wheel, click, header sorts"),
        ],
    ),
    (
        "ACTIONS",
        &[
            ("d", "describe"),
            ("m", "mailbox of an actor"),
            ("a  b", "a message's actor, cause"),
            ("P", "an actor's parent"),
            ("c", "cancel message"),
            ("r", "retry message"),
            ("n", "deliver message now"),
            ("p  u", "pause, resume actor"),
            ("x", "purge mailbox"),
            ("ctrl-k", "stop running turn"),
            ("ctrl-d", "delete actor, remove type"),
        ],
    ),
    ("DESCRIBE", &[("t", "yaml ⇄ json"), ("w", "wrap lines"), ("j k g G", "scroll")]),
];

fn help_lines(sections: &[Section]) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    for (title, keys) in sections {
        lines.push(Line::from(Span::styled(*title, Style::new().fg(LABEL).bold())));
        let width = keys.iter().map(|(k, _)| k.chars().count()).max().unwrap_or(0) + 2;
        for (key, text) in keys.iter() {
            lines.push(Line::from(vec![
                Span::styled(format!("{key:<width$}"), Style::new().fg(KEY).bold()),
                Span::styled(*text, Style::new().fg(Color::Gray)),
            ]));
        }
        lines.push(Line::from(""));
    }
    lines
}

fn draw_help(frame: &mut Frame) {
    let area = centered(frame.area(), 132, 20);
    frame.render_widget(Clear, area);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(ACCENT))
        .title(Line::from(" Help ".bold()).centered())
        .title_bottom(Line::from(" any key closes ".dim()).centered());
    let inner = block.inner(area).inner(Margin::new(1, 0));
    frame.render_widget(block, area);
    let widths = [Constraint::Length(46), Constraint::Length(42), Constraint::Fill(1)];
    let columns = Layout::horizontal(widths).spacing(2).split(inner);
    frame.render_widget(Paragraph::new(help_lines(&HELP[..1])), columns[0]);
    frame.render_widget(Paragraph::new(help_lines(&[HELP[1], HELP[3]])), columns[1]);
    frame.render_widget(Paragraph::new(help_lines(&HELP[2..3])), columns[2]);
}

#[cfg(test)]
mod tests {
    use cicada::ActorAddress;
    use cicada::api::{ActorView, StatsView};
    use cicada::store::MessageId;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use serde_json::json;

    use super::*;
    use crate::tui::app::{Effect, Fact, Fetched, Overview, ScreenData, Summary};
    use crate::tui::fetch;
    use crate::tui::grid::{GridRow, Rows};

    fn screen(app: &mut App, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|f| draw(f, app)).unwrap();
        let buffer = terminal.backend().buffer();
        (0..buffer.area.height)
            .map(|y| (0..buffer.area.width).map(|x| buffer[(x, y)].symbol()).collect::<String>() + "\n")
            .collect()
    }

    fn loaded(data: ScreenData) -> App {
        let mut app = App::new(Some("local".into()), "http://localhost:8080".into());
        let seq = app.start().iter().find_map(|e| if let Effect::Fetch { seq, .. } = e { Some(*seq) } else { None });
        app.on_result(Fetched::Screen { seq: seq.unwrap(), data });
        let stats = StatsView { actors: 11, pending: 4, oldest_due: None };
        app.on_result(Fetched::Summary(Ok(Summary { stats, nodes: vec![] })));
        app
    }

    fn outcome_rows() -> Rows {
        let row = |id: &str, status: &str| {
            let cells = vec![id.to_owned(), status.to_owned()];
            GridRow { item: Item::Message(MessageId::new(id).unwrap()), wide: cells.clone(), cells }
        };
        Rows {
            headers: vec!["ID", "STATUS"],
            wide_headers: vec!["ID", "STATUS"],
            rows: vec![row("f1", "failed"), row("d1", "done")],
            finished: true,
        }
    }

    #[test]
    fn the_list_screen_has_header_logo_table_and_crumbs() {
        let mut app = loaded(ScreenData::Grid(outcome_rows()));
        let text = screen(&mut app, 140, 24);
        let expected =
            ["Context: local", "http://localhost:8080", "11 actors · 4 pending", "nothing due", "<ctrl-d>", "╔═╗╦╔═╗"];
        for part in expected.into_iter().chain(["actors(all)[2]", "● failed", "<actors>"]) {
            assert!(text.contains(part), "{part} missing:\n{text}");
        }
        assert!(app.hit.is_some(), "the table records where it is");
    }

    #[test]
    fn a_failed_row_is_drawn_in_red() {
        let mut app = loaded(ScreenData::Grid(outcome_rows()));
        app.on_key(KeyEvent::from(KeyCode::Char('j'))); // select `d1`, so the failed row is not highlighted
        let mut terminal = Terminal::new(TestBackend::new(140, 24)).unwrap();
        terminal.draw(|f| draw(f, &mut app)).unwrap();
        let buffer = terminal.backend().buffer();
        // The cells of `word` in the table, by the column of its first letter in some row.
        let cells = |word: &str| {
            let width = word.chars().count() as u16;
            (0..buffer.area.height)
                .flat_map(|y| (0..buffer.area.width.saturating_sub(width)).map(move |x| (x, y)))
                .find(|&(x, y)| {
                    word.chars().enumerate().all(|(i, c)| buffer[(x + i as u16, y)].symbol() == c.to_string())
                })
                .map(|(x, y)| (x..x + width).map(|x| buffer[(x, y)].clone()).collect::<Vec<_>>())
                .unwrap_or_else(|| panic!("{word} is not drawn"))
        };
        for word in ["f1", "● failed"] {
            assert!(cells(word).iter().all(|c| c.fg == Color::Red), "{word} is red: {:?}", cells(word));
        }
        assert!(cells("d1").iter().all(|c| c.fg != Color::Red), "a done row is not red");
    }

    #[test]
    fn filter_prompt_confirm_and_help_draw() {
        let mut app = loaded(ScreenData::Grid(outcome_rows()));
        for c in "/f1".chars() {
            app.on_key(KeyEvent::from(KeyCode::Char(c)));
        }
        let text = screen(&mut app, 140, 24);
        assert!(text.contains(" filter ") && text.contains("</f1>") && text.contains("actors(all)[1]"), "{text}");

        app.mode = Mode::Confirm { action: Action::Purge(ActorAddress::parse("a/b").unwrap()), yes: false };
        let text = screen(&mut app, 140, 24);
        assert!(text.contains("Danger") && text.contains("Purge every pending message of actor a/b?"), "{text}");

        app.mode = Mode::Help;
        let text = screen(&mut app, 140, 30);
        assert!(text.contains("RESOURCES") && text.contains(":outcomes [status] [prefix]"), "{text}");
    }

    #[test]
    fn an_overview_shows_facts_pane_and_tabs() {
        let address = ActorAddress::parse("account/acme:7").unwrap();
        let overview = Overview {
            subject: Item::Actor(address.clone()),
            facts: vec![Fact { label: "Version", value: "3".into(), tone: Tone::Normal }],
            panes: vec![("state".into(), json!({"balance": 5}))],
            pane_follows_selection: false,
            caused_by: None,
            actor: None,
            parent: None,
        };
        let mut app = loaded(ScreenData::Grid(Rows::default()));
        let screen_state = &mut app.stack[0];
        screen_state.target = Target::Actor(address);
        screen_state.tabs = vec![("mailbox", Default::default()), ("outcomes", Default::default())];
        screen_state.overview = Some(overview);
        screen_state.tabs[1].1.set(outcome_rows());
        let text = screen(&mut app, 140, 30);
        for part in
            ["actor account/acme:7", "Version", "\"balance\": 5", "mailbox[0]", "outcomes[2]", "<account/acme:7>"]
        {
            assert!(text.contains(part), "{part} missing:\n{text}");
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::from(code)
    }

    /// The view of the actor `address` with `parent`, as the API serves it.
    fn view(address: &str, parent: Option<&str>) -> ActorView {
        let address = ActorAddress::parse(address).unwrap();
        ActorView {
            type_name: address.type_name().to_string(),
            address,
            state: json!({}),
            version: 0,
            created_at: chrono::Utc::now(),
            paused: false,
            parent: parent.map(|p| ActorAddress::parse(p).unwrap()),
        }
    }

    /// An app on the screen of `view` (opened with `:actor`) with the data the fetch builds, `children` among it.
    fn actor_screen_of(view: &ActorView, children: &[&str]) -> App {
        let mut app = App::new(Some("local".into()), "http://localhost:8080".into());
        app.start();
        for c in format!(":actor {}", view.address).chars() {
            app.on_key(key(KeyCode::Char(c)));
        }
        let effects = app.on_key(key(KeyCode::Enter));
        let seq = effects.iter().find_map(|e| if let Effect::Fetch { seq, .. } = e { Some(*seq) } else { None });
        let children: Vec<ActorAddress> = children.iter().map(|c| ActorAddress::parse(c).unwrap()).collect();
        let data = fetch::actor_screen(view, None, &[], &[], &children);
        app.on_result(Fetched::Screen { seq: seq.expect("a fetch"), data });
        app
    }

    fn opens_actor(effects: &[Effect], address: &str) -> bool {
        matches!(effects, [Effect::Fetch { target: Target::Actor(a), .. }] if a.as_str() == address)
    }

    /// Scenario: Up to the parent.
    #[test]
    fn the_screen_of_a_child_names_its_parent_and_p_opens_it() {
        let mut app = actor_screen_of(&view("batch-items/b1:0", Some("batches/b1")), &[]);
        let text = screen(&mut app, 140, 30);
        assert!(text.lines().any(|l| l.contains("Parent") && l.contains("batches/b1")), "{text}");
        assert!(text.contains("<P>") && text.contains("children[0]"), "{text}");
        assert!(opens_actor(&app.on_key(key(KeyCode::Char('P'))), "batches/b1"));
        assert_eq!(app.stack.iter().map(|s| s.target.crumb()).collect::<Vec<_>>(), ["batch-items/b1:0", "batches/b1"]);
        assert!(opens_actor(&app.on_key(key(KeyCode::Esc)), "batch-items/b1:0"), "esc goes back");
    }

    /// Scenario: Down to a child.
    #[test]
    fn the_children_tab_lists_the_children_in_address_order_and_enter_opens_one() {
        let mut app = actor_screen_of(&view("batches/b1", None), &["batch-items/b1:0", "batch-items/b1:1"]);
        app.on_key(key(KeyCode::Tab));
        app.on_key(key(KeyCode::Tab));
        let text = screen(&mut app, 140, 30);
        assert!(text.contains("children[2]"), "{text}");
        let (first, second) = (text.find("batch-items/b1:0"), text.find("batch-items/b1:1"));
        assert!(first.is_some() && first < second, "both, in address order:\n{text}");
        app.on_key(key(KeyCode::Char('j')));
        assert!(opens_actor(&app.on_key(key(KeyCode::Enter)), "batch-items/b1:1"));
        assert!(opens_actor(&app.on_key(key(KeyCode::Esc)), "batches/b1"), "esc goes back");
    }

    /// Scenario: An actor on its own.
    #[test]
    fn an_independent_actor_without_children_has_no_parent_and_p_does_nothing() {
        let mut app = actor_screen_of(&view("solo/x", None), &[]);
        let text = screen(&mut app, 140, 30);
        assert!(text.lines().any(|l| l.contains("Parent") && l.contains("none")), "{text}");
        assert!(text.contains("children[0]"), "{text}");
        assert!(app.on_key(key(KeyCode::Char('P'))).is_empty());
        assert_eq!(app.stack.len(), 1);

        // The question before a delete says that the descendants go too.
        app.on_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL));
        let text = screen(&mut app, 140, 30);
        assert!(text.contains("Delete actor solo/x with all its descendants"), "{text}");
    }
}
