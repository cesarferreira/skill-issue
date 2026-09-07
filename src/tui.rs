use super::{
    Config, InstallationKind, ScanResult, SkillDeletePlan, SkillGroup, SkillStatus,
    SkillTogglePlan, TuiSyncPlan, config_with_project, plan_skill_delete, plan_skill_toggle,
    plan_tui_sync, scan,
};
use anyhow::{Result, bail};
use crossterm::{
    event::{
        self, DisableFocusChange, EnableFocusChange, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Frame, Terminal,
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Layout, Margin, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{
        Block, Borders, Cell, Clear, Gauge, List, ListItem, ListState, Padding, Paragraph, Row,
        Table, TableState, Tabs, Wrap,
    },
};
use std::{
    io::{self, IsTerminal},
    path::PathBuf,
    time::{Duration, Instant},
};

const LIVE_REFRESH_INTERVAL: Duration = Duration::from_secs(1);

const BRAND: Color = Color::Rgb(125, 211, 252);
const ACCENT: Color = Color::Rgb(167, 139, 250);
const GOOD: Color = Color::Rgb(74, 222, 128);
const WARN: Color = Color::Rgb(250, 204, 21);
const BAD: Color = Color::Rgb(248, 113, 113);
const MUTED: Color = Color::Rgb(148, 163, 184);

pub(super) fn run(
    config: Config,
    project: Option<PathBuf>,
    dry_run: bool,
    no_color: bool,
) -> Result<u8> {
    if super::JSON_OUTPUT.load(std::sync::atomic::Ordering::Relaxed) {
        bail!("--json cannot be combined with tui");
    }
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        bail!("the TUI requires an interactive terminal");
    }
    let effective = config_with_project(&config, project.as_deref())?;
    let app = App::new(effective, dry_run, no_color)?;
    run_terminal(app)?;
    Ok(super::EXIT_OK)
}

fn run_terminal(mut app: App) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    if let Err(error) = execute!(stdout, EnterAlternateScreen, EnableFocusChange) {
        let _ = disable_raw_mode();
        return Err(error.into());
    }
    let _restore = RestoreTerminal;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = app.event_loop(&mut terminal);
    let _ = terminal.show_cursor();
    result
}

struct RestoreTerminal;

impl Drop for RestoreTerminal {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), DisableFocusChange, LeaveAlternateScreen);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Skills,
    Agents,
    Health,
}

impl View {
    const ALL: [Self; 3] = [Self::Skills, Self::Agents, Self::Health];

    fn index(self) -> usize {
        match self {
            Self::Skills => 0,
            Self::Agents => 1,
            Self::Health => 2,
        }
    }

    fn next(self, reverse: bool) -> Self {
        let offset = if reverse { 2 } else { 1 };
        Self::ALL[(self.index() + offset) % Self::ALL.len()]
    }
}

enum Mode {
    Browse,
    Filter,
    Help,
    Confirm(SkillTogglePlan),
    ConfirmDelete(SkillDeletePlan),
    ConfirmSync(TuiSyncPlan),
    Notice,
}

struct App {
    config: Config,
    result: ScanResult,
    view: View,
    skill_state: TableState,
    agent_state: ListState,
    filter: String,
    mode: Mode,
    notice: String,
    dry_run: bool,
    no_color: bool,
    should_quit: bool,
    last_refresh: Instant,
    focused: bool,
}

impl App {
    fn new(config: Config, dry_run: bool, no_color: bool) -> Result<Self> {
        let result = scan(&config)?;
        let mut skill_state = TableState::default();
        if !result.groups.is_empty() {
            skill_state.select(Some(0));
        }
        let mut agent_state = ListState::default();
        if !config.targets.is_empty() {
            agent_state.select(Some(0));
        }
        Ok(Self {
            config,
            result,
            view: View::Skills,
            skill_state,
            agent_state,
            filter: String::new(),
            mode: Mode::Browse,
            notice: String::new(),
            dry_run,
            no_color,
            should_quit: false,
            last_refresh: Instant::now(),
            focused: true,
        })
    }

    fn event_loop<B: Backend>(&mut self, terminal: &mut Terminal<B>) -> Result<()> {
        while !self.should_quit {
            terminal.draw(|frame| self.draw(frame))?;
            let timeout = if self.focused {
                LIVE_REFRESH_INTERVAL.saturating_sub(self.last_refresh.elapsed())
            } else {
                // Unfocused: no live refresh to race against, so block until the
                // next input/focus event instead of waking up every second.
                Duration::from_secs(3600)
            };
            if event::poll(timeout)? {
                match event::read()? {
                    Event::Key(key) if key.kind == KeyEventKind::Press => {
                        self.handle_key(key)?;
                    }
                    Event::FocusGained => {
                        self.focused = true;
                        self.live_refresh();
                    }
                    Event::FocusLost => self.focused = false,
                    _ => {}
                }
            }
            if self.focused && self.last_refresh.elapsed() >= LIVE_REFRESH_INTERVAL {
                self.live_refresh();
            }
        }
        Ok(())
    }

    fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        match &self.mode {
            Mode::Filter => return self.handle_filter(key),
            Mode::Help | Mode::Notice => {
                self.mode = Mode::Browse;
                return Ok(());
            }
            Mode::Confirm(_) => {
                return self.handle_confirmation(key);
            }
            Mode::ConfirmDelete(_) => {
                return self.handle_delete_confirmation(key);
            }
            Mode::ConfirmSync(_) => {
                return self.handle_sync_confirmation(key);
            }
            Mode::Browse => {}
        }

        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => self.should_quit = true,
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.should_quit = true
            }
            KeyCode::Tab => self.view = self.view.next(false),
            KeyCode::BackTab => self.view = self.view.next(true),
            KeyCode::Char('?') => self.mode = Mode::Help,
            KeyCode::Char('/') if self.view == View::Skills => self.mode = Mode::Filter,
            KeyCode::Char('r') => self.refresh()?,
            KeyCode::Char('s') => self.prepare_sync()?,
            KeyCode::Char('d') if self.view == View::Skills => self.prepare_toggle()?,
            KeyCode::Char('x' | 'D') if self.view == View::Skills => self.prepare_delete()?,
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(-1),
            KeyCode::Home => self.select_edge(false),
            KeyCode::End => self.select_edge(true),
            _ => {}
        }
        Ok(())
    }

    fn handle_filter(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Esc => {
                self.filter.clear();
                self.normalize_skill_selection();
                self.mode = Mode::Browse;
            }
            KeyCode::Enter => self.mode = Mode::Browse,
            KeyCode::Backspace => {
                self.filter.pop();
                self.normalize_skill_selection();
            }
            KeyCode::Char(character)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.filter.push(character);
                self.normalize_skill_selection();
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_confirmation(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let Mode::Confirm(plan) = std::mem::replace(&mut self.mode, Mode::Browse) else {
                    return Ok(());
                };
                if self.dry_run {
                    self.notice = format!(
                        "Dry run: {} would be {} for {} agent{}.",
                        plan.skill,
                        if plan.enable { "enabled" } else { "disabled" },
                        plan.action_count(),
                        if plan.action_count() == 1 { "" } else { "s" }
                    );
                } else {
                    plan.apply()?;
                    self.notice = format!(
                        "{} is now {} for {} agent{}.",
                        plan.skill,
                        if plan.enable { "enabled" } else { "disabled" },
                        plan.action_count(),
                        if plan.action_count() == 1 { "" } else { "s" }
                    );
                    self.refresh()?;
                }
                self.mode = Mode::Notice;
            }
            KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::Browse,
            _ => {}
        }
        Ok(())
    }

    fn handle_delete_confirmation(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let Mode::ConfirmDelete(plan) = std::mem::replace(&mut self.mode, Mode::Browse)
                else {
                    return Ok(());
                };
                if self.dry_run {
                    self.notice = format!(
                        "Dry run: {} would be permanently deleted ({} link{} removed).",
                        plan.skill,
                        plan.link_count(),
                        if plan.link_count() == 1 { "" } else { "s" }
                    );
                } else {
                    plan.apply()?;
                    self.notice = format!("{} was permanently deleted.", plan.skill);
                    self.refresh()?;
                }
                self.mode = Mode::Notice;
            }
            KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::Browse,
            _ => {}
        }
        Ok(())
    }

    fn refresh(&mut self) -> Result<()> {
        self.result = scan(&self.config)?;
        self.normalize_skill_selection();
        self.last_refresh = Instant::now();
        Ok(())
    }

    fn live_refresh(&mut self) {
        if let Ok(result) = scan(&self.config) {
            self.result = result;
            self.normalize_skill_selection();
        }
        self.last_refresh = Instant::now();
    }

    fn prepare_sync(&mut self) -> Result<()> {
        match plan_tui_sync(&self.config) {
            Ok(plan) if !plan.conflicts().is_empty() => {
                let (path, reason) = &plan.conflicts()[0];
                self.notice = format!("Cannot sync: {} {reason}", super::theme::display_path(path));
                self.mode = Mode::Notice;
            }
            Ok(plan) if plan.is_empty() => {
                self.notice = "Everything is already synchronized.".to_string();
                self.mode = Mode::Notice;
            }
            Ok(plan) => self.mode = Mode::ConfirmSync(plan),
            Err(error) => {
                self.notice = format!("Cannot sync: {error:#}");
                self.mode = Mode::Notice;
            }
        }
        Ok(())
    }

    fn handle_sync_confirmation(&mut self, key: KeyEvent) -> Result<()> {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                let Mode::ConfirmSync(plan) = std::mem::replace(&mut self.mode, Mode::Browse)
                else {
                    return Ok(());
                };
                let actions = plan.action_count();
                if self.dry_run {
                    self.notice = format!(
                        "Dry run: sync would perform {actions} filesystem action{}.",
                        if actions == 1 { "" } else { "s" }
                    );
                } else {
                    let actions = plan.apply(&self.config)?;
                    self.refresh()?;
                    self.notice = format!(
                        "Sync complete: {actions} filesystem action{} applied.",
                        if actions == 1 { "" } else { "s" }
                    );
                }
                self.mode = Mode::Notice;
            }
            KeyCode::Char('n') | KeyCode::Esc => self.mode = Mode::Browse,
            _ => {}
        }
        Ok(())
    }

    fn prepare_toggle(&mut self) -> Result<()> {
        let Some(group) = self.selected_skill() else {
            self.notice = "No canonical skill selected.".to_string();
            self.mode = Mode::Notice;
            return Ok(());
        };
        if group.canonical.is_none() {
            self.notice = format!(
                "{} is not collected yet. Collect it with `si setup`.",
                group.name
            );
            self.mode = Mode::Notice;
            return Ok(());
        }
        let enabled = group
            .installations
            .iter()
            .any(|installation| installation.kind == InstallationKind::ManagedSymlink);
        match plan_skill_toggle(&self.config, &group.name, !enabled) {
            Ok(plan) if plan.is_empty() => {
                self.notice = format!(
                    "{} is already {}.",
                    group.name,
                    if enabled { "enabled" } else { "disabled" }
                );
                self.mode = Mode::Notice;
            }
            Ok(plan) => self.mode = Mode::Confirm(plan),
            Err(error) => {
                self.notice = format!("{error:#}");
                self.mode = Mode::Notice;
            }
        }
        Ok(())
    }

    fn prepare_delete(&mut self) -> Result<()> {
        let Some(group) = self.selected_skill() else {
            self.notice = "No canonical skill selected.".to_string();
            self.mode = Mode::Notice;
            return Ok(());
        };
        if group.canonical.is_none() {
            self.notice = format!("{} is not canonical yet; nothing to delete.", group.name);
            self.mode = Mode::Notice;
            return Ok(());
        }
        match plan_skill_delete(&self.config, &group.name) {
            Ok(plan) => self.mode = Mode::ConfirmDelete(plan),
            Err(error) => {
                self.notice = format!("{error:#}");
                self.mode = Mode::Notice;
            }
        }
        Ok(())
    }

    fn move_selection(&mut self, delta: isize) {
        if self.view == View::Agents {
            let len = self.config.targets.len();
            if len == 0 {
                self.agent_state.select(None);
                return;
            }
            let current = self.agent_state.selected().unwrap_or(0);
            self.agent_state.select(Some(
                (current as isize + delta).rem_euclid(len as isize) as usize
            ));
        } else {
            let len = self.filtered_skills().len();
            if len == 0 {
                self.skill_state.select(None);
                return;
            }
            let current = self.skill_state.selected().unwrap_or(0);
            self.skill_state.select(Some(
                (current as isize + delta).rem_euclid(len as isize) as usize
            ));
        }
    }

    fn select_edge(&mut self, end: bool) {
        if self.view == View::Agents {
            let len = self.config.targets.len();
            self.agent_state.select(if len == 0 {
                None
            } else {
                Some(if end { len - 1 } else { 0 })
            });
        } else {
            let len = self.filtered_skills().len();
            self.skill_state.select(if len == 0 {
                None
            } else {
                Some(if end { len - 1 } else { 0 })
            });
        }
    }

    fn normalize_skill_selection(&mut self) {
        let len = self.filtered_skills().len();
        self.skill_state.select(if len == 0 {
            None
        } else {
            Some(self.skill_state.selected().unwrap_or(0).min(len - 1))
        });
    }

    fn filtered_skills(&self) -> Vec<&SkillGroup> {
        let needle = self.filter.to_lowercase();
        self.result
            .groups
            .values()
            .filter(|group| needle.is_empty() || group.name.to_lowercase().contains(&needle))
            .collect()
    }

    fn enabled_target_count(&self) -> usize {
        self.config
            .targets
            .values()
            .filter(|target| target.enabled)
            .count()
    }

    fn selected_skill(&self) -> Option<&SkillGroup> {
        self.skill_state
            .selected()
            .and_then(|index| self.filtered_skills().get(index).copied())
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        frame.render_widget(
            Block::default().style(self.style(Color::Rgb(15, 23, 42))),
            area,
        );
        let chunks = Layout::vertical([
            Constraint::Length(if area.width < 100 { 5 } else { 3 }),
            Constraint::Min(12),
            Constraint::Length(if matches!(self.mode, Mode::Filter) {
                4
            } else {
                3
            }),
        ])
        .split(area);
        self.draw_header(frame, chunks[0]);
        match self.view {
            View::Skills => self.draw_skills(frame, chunks[1]),
            View::Agents => self.draw_agents(frame, chunks[1]),
            View::Health => self.draw_health(frame, chunks[1]),
        }
        self.draw_footer(frame, chunks[2]);
        self.draw_overlay(frame);
    }

    fn draw_header(&self, frame: &mut Frame, area: Rect) {
        let count = |status| {
            self.result
                .groups
                .values()
                .filter(|group| group.status() == status)
                .count()
        };
        let managed = count(SkillStatus::Managed);
        let total = self.result.groups.len();
        let conflicts = count(SkillStatus::Divergent);
        let broken = count(SkillStatus::Broken);
        let pending = count(SkillStatus::Unique) + count(SkillStatus::IdenticalDuplicate);
        let title = Line::from(vec![
            Span::styled(" ◆ ", self.style(BRAND).add_modifier(Modifier::BOLD)),
            Span::styled(
                "skill-issue",
                self.style(Color::White).add_modifier(Modifier::BOLD),
            ),
            Span::styled("  SKILL CONTROL CENTER", self.style(MUTED)),
        ]);
        let summary = Line::from(vec![
            Span::styled(format!(" {managed} managed "), self.pill(GOOD)),
            Span::raw(" "),
            Span::styled(format!(" {broken} broken "), self.pill(WARN)),
            Span::raw(" "),
            Span::styled(format!(" {conflicts} conflicts "), self.pill(BAD)),
            Span::raw(" "),
            Span::styled(format!(" {pending} pending "), self.pill(ACCENT)),
        ]);
        let summary_health = Line::from(vec![
            Span::styled(format!(" {managed} managed "), self.pill(GOOD)),
            Span::raw("  "),
            Span::styled(format!(" {broken} broken "), self.pill(WARN)),
        ]);
        let summary_issues = Line::from(vec![
            Span::styled(format!(" {conflicts} conflicts "), self.pill(BAD)),
            Span::raw("  "),
            Span::styled(format!(" {pending} pending "), self.pill(ACCENT)),
        ]);
        let tabs = Tabs::new(View::ALL.map(|view| {
            let label = match view {
                View::Skills => "SKILLS",
                View::Agents => "AGENTS",
                View::Health => "HEALTH",
            };
            if view == self.view {
                format!("[ {label} ]")
            } else {
                format!("  {label}  ")
            }
        }))
        .select(self.view.index())
        .style(self.style(MUTED))
        .highlight_style(self.pill(BRAND))
        .divider(Span::styled(" │ ", self.style(MUTED)));
        let context = Paragraph::new(format!(
            "{total} skills  ·  {} agents",
            self.enabled_target_count()
        ))
        .style(self.style(MUTED))
        .alignment(Alignment::Right);
        let inner = area.inner(Margin {
            horizontal: 1,
            vertical: 0,
        });
        if area.width < 100 {
            let rows = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
            frame.render_widget(title, rows[0]);
            frame.render_widget(summary_health, rows[1]);
            frame.render_widget(summary_issues, rows[2]);
            if area.width >= 70 {
                let navigation = Layout::horizontal([Constraint::Min(40), Constraint::Length(24)])
                    .split(rows[3]);
                frame.render_widget(tabs, navigation[0]);
                frame.render_widget(context, navigation[1]);
            } else {
                frame.render_widget(tabs, rows[3]);
            }
        } else {
            let rows =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);
            let top =
                Layout::horizontal([Constraint::Min(20), Constraint::Length(55)]).split(rows[0]);
            frame.render_widget(title, top[0]);
            frame.render_widget(Paragraph::new(summary).alignment(Alignment::Right), top[1]);
            let navigation =
                Layout::horizontal([Constraint::Min(40), Constraint::Length(24)]).split(rows[1]);
            frame.render_widget(tabs, navigation[0]);
            frame.render_widget(context, navigation[1]);
        }
    }

    fn draw_skills(&mut self, frame: &mut Frame, area: Rect) {
        let panes = Layout::horizontal([Constraint::Percentage(58), Constraint::Percentage(42)])
            .split(area);
        let rows: Vec<Row> = self
            .filtered_skills()
            .iter()
            .map(|group| {
                let (label, color) = visual_status(group);
                let linked = group
                    .installations
                    .iter()
                    .filter(|item| item.kind == InstallationKind::ManagedSymlink)
                    .count();
                Row::new(vec![
                    Cell::from(format!(" {label}")),
                    Cell::from(group.name.clone()),
                    Cell::from(if group.canonical.is_some() {
                        "yes"
                    } else {
                        "—"
                    }),
                    Cell::from(format!("{linked}/{}", self.enabled_target_count())),
                ])
                .style(self.style(color))
            })
            .collect();
        let table = Table::new(
            rows,
            [
                Constraint::Length(12),
                Constraint::Min(14),
                Constraint::Length(10),
                Constraint::Length(9),
            ],
        )
        .header(
            Row::new([" STATUS", "SKILL", "CANONICAL", "AGENTS"])
                .style(self.style(MUTED).add_modifier(Modifier::BOLD)),
        )
        .row_highlight_style(
            self.style(Color::White)
                .bg(if self.no_color {
                    Color::DarkGray
                } else {
                    Color::Rgb(51, 65, 85)
                })
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▌")
        .block(self.panel(" Skills "));
        frame.render_stateful_widget(table, panes[0], &mut self.skill_state);
        self.draw_skill_detail(frame, panes[1]);
    }

    fn draw_skill_detail(&self, frame: &mut Frame, area: Rect) {
        let Some(group) = self.selected_skill() else {
            frame.render_widget(
                Paragraph::new("No skills match this filter.")
                    .alignment(Alignment::Center)
                    .block(self.panel(" Details ")),
                area,
            );
            return;
        };
        let (status, color) = visual_status(group);
        let canonical = group
            .canonical
            .as_ref()
            .map(|skill| super::theme::display_path(&skill.path))
            .unwrap_or_else(|| "Not adopted".to_string());
        let mut lines = vec![
            Line::styled(
                group.name.clone(),
                self.style(Color::White).add_modifier(Modifier::BOLD),
            ),
            Line::from(vec![
                Span::styled(format!(" {status} "), self.pill(color)),
                Span::raw("  "),
                Span::styled(
                    format!("{} installation(s)", group.installations.len()),
                    self.style(MUTED),
                ),
            ]),
            Line::raw(""),
            Line::styled("CANONICAL", self.style(MUTED).add_modifier(Modifier::BOLD)),
            Line::styled(canonical, self.style(BRAND)),
            Line::raw(""),
            Line::styled(
                "INSTALLATIONS",
                self.style(MUTED).add_modifier(Modifier::BOLD),
            ),
        ];
        if group.installations.is_empty() {
            lines.push(Line::styled(
                "Not exposed to any configured agent",
                self.style(MUTED),
            ));
        } else {
            for installation in &group.installations {
                let (kind, color) = installation_style(&installation.kind);
                lines.push(Line::from(vec![
                    Span::styled(format!("● {:<12}", installation.target), self.style(color)),
                    Span::styled(kind, self.style(MUTED)),
                ]));
                lines.push(Line::styled(
                    format!("  {}", super::theme::display_path(&installation.path)),
                    self.style(MUTED),
                ));
            }
        }
        lines.push(Line::raw(""));
        lines.push(Line::styled(
            if group.canonical.is_some() {
                "d  toggle enable / disable"
            } else {
                "Collect with: si setup"
            },
            self.style(ACCENT),
        ));
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(self.panel(" Details ").padding(Padding::uniform(1))),
            area,
        );
    }

    fn draw_agents(&mut self, frame: &mut Frame, area: Rect) {
        let panes = Layout::horizontal([Constraint::Percentage(42), Constraint::Percentage(58)])
            .split(area);
        let items: Vec<ListItem> = self
            .config
            .targets
            .iter()
            .map(|(id, target)| {
                let healthy = target.path.is_dir();
                ListItem::new(Line::from(vec![
                    Span::styled("● ", self.style(if healthy { GOOD } else { WARN })),
                    Span::styled(id.clone(), self.style(Color::White)),
                ]))
            })
            .collect();
        let list = List::new(items)
            .block(self.panel(" Configured agents "))
            .highlight_symbol("▌")
            .highlight_style(self.style(Color::White).bg(if self.no_color {
                Color::DarkGray
            } else {
                Color::Rgb(51, 65, 85)
            }));
        frame.render_stateful_widget(list, panes[0], &mut self.agent_state);

        let detail = self
            .agent_state
            .selected()
            .and_then(|index| self.config.targets.iter().nth(index));
        let lines = if let Some((id, target)) = detail {
            let installed = self.result.target_counts.get(id).copied().unwrap_or(0);
            let managed = self
                .result
                .groups
                .values()
                .flat_map(|group| &group.installations)
                .filter(|item| item.target == *id && item.kind == InstallationKind::ManagedSymlink)
                .count();
            vec![
                Line::styled(
                    id.clone(),
                    self.style(Color::White).add_modifier(Modifier::BOLD),
                ),
                Line::raw(""),
                Line::styled("PATH", self.style(MUTED)),
                Line::styled(super::theme::display_path(&target.path), self.style(BRAND)),
                Line::raw(""),
                Line::from(format!(
                    "State          {}",
                    if target.enabled {
                        "enabled"
                    } else {
                        "disabled"
                    }
                )),
                Line::from(format!("Installations  {installed}")),
                Line::from(format!("Managed links  {managed}")),
                Line::raw(""),
                Line::styled(
                    "Manage agents with `si targets add|remove`.",
                    self.style(MUTED),
                ),
            ]
        } else {
            vec![Line::raw("No configured agents.")]
        };
        frame.render_widget(
            Paragraph::new(lines)
                .wrap(Wrap { trim: false })
                .block(self.panel(" Agent details ").padding(Padding::uniform(1))),
            panes[1],
        );
    }

    fn draw_health(&self, frame: &mut Frame, area: Rect) {
        let inner = area.inner(Margin {
            horizontal: 2,
            vertical: 1,
        });
        let rows = Layout::vertical([
            Constraint::Length(3),
            Constraint::Length(2),
            Constraint::Min(5),
        ])
        .split(inner);
        let total = self.result.groups.len();
        let managed = self
            .result
            .groups
            .values()
            .filter(|group| group.status() == SkillStatus::Managed)
            .count();
        let ratio = if total == 0 {
            1.0
        } else {
            managed as f64 / total as f64
        };
        let gauge_area = rows[0];
        frame.render_widget(
            Gauge::default()
                .block(self.panel(" Canonical health "))
                .gauge_style(self.style(GOOD).add_modifier(Modifier::BOLD))
                .ratio(ratio)
                .label(""),
            gauge_area,
        );
        let available = gauge_area.width.saturating_sub(2);
        let full_label = format!(" {managed}/{total} fully managed ");
        let label = if full_label.chars().count() as u16 <= available {
            full_label
        } else {
            format!(" {managed}/{total} ")
        };
        let label_width = (label.chars().count() as u16).min(available);
        if label_width > 0 && gauge_area.height > 2 {
            let label_area = Rect::new(
                gauge_area.x + gauge_area.width.saturating_sub(label_width) / 2,
                gauge_area.y + gauge_area.height / 2,
                label_width,
                1,
            );
            let label_style = if self.no_color {
                Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)
            } else {
                Style::default()
                    .fg(Color::White)
                    .bg(Color::Rgb(15, 23, 42))
                    .add_modifier(Modifier::BOLD)
            };
            frame.render_widget(Paragraph::new(label).style(label_style), label_area);
        }
        let root_state = if self.config.root.is_dir() {
            ("● canonical root is healthy", GOOD)
        } else {
            ("● canonical root is missing", BAD)
        };
        frame.render_widget(
            Paragraph::new(Line::styled(root_state.0, self.style(root_state.1))),
            rows[1],
        );
        let mut issues = Vec::new();
        for target in &self.result.missing_targets {
            issues.push(ListItem::new(format!(
                "⚠ {target}: target directory is missing"
            )));
        }
        for group in self.result.groups.values() {
            if !matches!(group.status(), SkillStatus::Managed) {
                let (status, _) = visual_status(group);
                issues.push(ListItem::new(format!("• {}  {status}", group.name)));
            }
        }
        if issues.is_empty() {
            issues.push(ListItem::new("✓ No skill issues."));
        }
        frame.render_widget(
            List::new(issues)
                .style(self.style(if managed == total { GOOD } else { WARN }))
                .block(self.panel(" Diagnostics ").padding(Padding::uniform(1))),
            rows[2],
        );
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let text = if matches!(self.mode, Mode::Filter) {
            vec![
                Line::styled(
                    format!(" / {}_", self.filter),
                    self.style(Color::White).add_modifier(Modifier::BOLD),
                ),
                Line::styled(" Enter sync  ·  Esc clear", self.style(MUTED)),
            ]
        } else {
            vec![Line::from(vec![
                Span::styled(" ↑↓/jk ", self.pill(MUTED)),
                Span::styled(" navigate  ", self.style(Color::White)),
                Span::styled(" Tab ", self.pill(BRAND)),
                Span::styled(" views  ", self.style(Color::White)),
                Span::styled(" / ", self.pill(ACCENT)),
                Span::styled(" filter  ", self.style(Color::White)),
                Span::styled(" d ", self.pill(WARN)),
                Span::styled(" toggle  ", self.style(Color::White)),
                Span::styled(" x ", self.pill(BAD)),
                Span::styled(" delete  ", self.style(Color::White)),
                Span::styled(" r ", self.pill(GOOD)),
                Span::styled(" refresh  ", self.style(Color::White)),
                Span::styled(" s ", self.pill(ACCENT)),
                Span::styled(" sync  ", self.style(Color::White)),
                Span::styled(" ? ", self.pill(MUTED)),
                Span::styled(" help  ", self.style(Color::White)),
                Span::styled(" q ", self.pill(BAD)),
                Span::styled(" quit", self.style(Color::White)),
            ])]
        };
        frame.render_widget(
            Paragraph::new(text).block(
                Block::default()
                    .borders(Borders::TOP)
                    .border_style(self.style(MUTED)),
            ),
            area,
        );
    }

    fn draw_overlay(&self, frame: &mut Frame) {
        let (title, body, width, height) = match &self.mode {
            Mode::Help => (
                " Keyboard guide ",
                vec![
                    Line::styled("NAVIGATION", self.style(BRAND).add_modifier(Modifier::BOLD)),
                    Line::raw("↑/↓ or j/k   Move selection"),
                    Line::raw("Tab / ⇧Tab   Change dashboard view"),
                    Line::raw("/            Filter skills"),
                    Line::raw("r            Rescan the filesystem"),
                    Line::raw("s            Preview and synchronize all skills"),
                    Line::raw(""),
                    Line::styled("ACTIONS", self.style(ACCENT).add_modifier(Modifier::BOLD)),
                    Line::raw("d            Enable or disable the selected skill"),
                    Line::raw("             (always previews and confirms first)"),
                    Line::raw(""),
                    Line::styled(
                        "x / D        Permanently delete the selected skill",
                        self.style(BAD),
                    ),
                    Line::raw("             (removes every link and the canonical copy)"),
                    Line::raw(""),
                    Line::styled("Press any key to close", self.style(MUTED)),
                ],
                62,
                20,
            ),
            Mode::Confirm(plan) => {
                let action = if plan.enable { "ENABLE" } else { "DISABLE" };
                let mut lines = vec![
                    Line::styled(
                        format!("{action} {}", plan.skill),
                        self.style(if plan.enable { GOOD } else { WARN })
                            .add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(""),
                    Line::raw(format!(
                        "{} managed link{} will be {}.",
                        plan.action_count(),
                        if plan.action_count() == 1 { "" } else { "s" },
                        if plan.enable { "created" } else { "removed" }
                    )),
                ];
                for path in plan.paths().take(5) {
                    lines.push(Line::styled(
                        format!("  {}", super::theme::display_path(path)),
                        self.style(MUTED),
                    ));
                }
                if plan.action_count() > 5 {
                    lines.push(Line::styled(
                        format!("  … and {} more", plan.action_count() - 5),
                        self.style(MUTED),
                    ));
                }
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    if self.dry_run {
                        "DRY RUN — no files will change"
                    } else {
                        "The canonical copy will never be deleted."
                    },
                    self.style(BRAND),
                ));
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    "Enter/y confirm  ·  n/Esc cancel",
                    self.style(ACCENT),
                ));
                (" Confirm action ", lines, 72, 15)
            }
            Mode::ConfirmDelete(plan) => {
                let mut lines = vec![
                    Line::styled(
                        format!("DELETE {}", plan.skill),
                        self.style(BAD).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(""),
                    Line::raw(format!(
                        "{} link{} and the canonical copy will be removed.",
                        plan.link_count(),
                        if plan.link_count() == 1 { "" } else { "s" }
                    )),
                ];
                for path in plan.links().take(5) {
                    lines.push(Line::styled(
                        format!("  {}", super::theme::display_path(path)),
                        self.style(MUTED),
                    ));
                }
                if plan.link_count() > 5 {
                    lines.push(Line::styled(
                        format!("  … and {} more", plan.link_count() - 5),
                        self.style(MUTED),
                    ));
                }
                lines.push(Line::styled(
                    format!("  {}", super::theme::display_path(plan.canonical())),
                    self.style(MUTED),
                ));
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    if self.dry_run {
                        "DRY RUN — no files will change"
                    } else {
                        "This cannot be undone."
                    },
                    self.style(BAD),
                ));
                lines.push(Line::raw(""));
                lines.push(Line::styled(
                    "Enter/y confirm  ·  n/Esc cancel",
                    self.style(ACCENT),
                ));
                (" Confirm delete ", lines, 72, 16)
            }
            Mode::ConfirmSync(plan) => {
                let mut lines = vec![
                    Line::styled(
                        "SYNC ALL SKILLS",
                        self.style(ACCENT).add_modifier(Modifier::BOLD),
                    ),
                    Line::raw(""),
                    Line::raw(format!(
                        "{} skill{} will be collected; up to {} filesystem action{} planned.",
                        plan.skill_count(),
                        if plan.skill_count() == 1 { "" } else { "s" },
                        plan.action_count(),
                        if plan.action_count() == 1 { "" } else { "s" },
                    )),
                    Line::raw(""),
                    Line::styled(
                        if self.dry_run {
                            "DRY RUN — no files will change"
                        } else {
                            "Only unambiguous changes will be applied."
                        },
                        self.style(BRAND),
                    ),
                    Line::raw(""),
                    Line::styled("Enter/y confirm  ·  n/Esc cancel", self.style(ACCENT)),
                ];
                if plan.skill_count() == 0 {
                    lines[2] = Line::raw(format!(
                        "{} reconciliation action{} planned.",
                        plan.action_count(),
                        if plan.action_count() == 1 { "" } else { "s" },
                    ));
                }
                (" Confirm sync ", lines, 72, 12)
            }
            Mode::Notice => (
                " skill-issue ",
                vec![
                    Line::raw(self.notice.clone()),
                    Line::raw(""),
                    Line::styled("Press any key to continue", self.style(MUTED)),
                ],
                68,
                9,
            ),
            _ => return,
        };
        let area = centered(frame.area(), width, height);
        frame.render_widget(Clear, area);
        frame.render_widget(
            Paragraph::new(body)
                .alignment(Alignment::Left)
                .wrap(Wrap { trim: false })
                .block(
                    Block::default()
                        .title(title)
                        .title_alignment(Alignment::Center)
                        .borders(Borders::ALL)
                        .border_style(self.style(BRAND))
                        .style(if self.no_color {
                            Style::default()
                        } else {
                            self.style(Color::White).bg(Color::Rgb(15, 23, 42))
                        })
                        .padding(Padding::uniform(2)),
                ),
            area,
        );
    }

    fn panel<'a>(&self, title: &'a str) -> Block<'a> {
        Block::default()
            .title(title)
            .borders(Borders::ALL)
            .border_style(self.style(Color::Rgb(51, 65, 85)))
            .style(self.style(Color::White))
    }

    fn style(&self, color: Color) -> Style {
        if self.no_color {
            Style::default()
        } else {
            Style::default().fg(color)
        }
    }

    fn pill(&self, color: Color) -> Style {
        if self.no_color {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
                .fg(Color::Rgb(15, 23, 42))
                .bg(color)
                .add_modifier(Modifier::BOLD)
        }
    }
}

fn visual_status(group: &SkillGroup) -> (&'static str, Color) {
    if group.canonical.is_some() && group.installations.is_empty() {
        return ("DISABLED", MUTED);
    }
    match group.status() {
        SkillStatus::Managed => ("MANAGED", GOOD),
        SkillStatus::IdenticalDuplicate => ("DUPLICATE", WARN),
        SkillStatus::Divergent => ("CONFLICT", BAD),
        SkillStatus::Unique => ("UNADOPTED", BRAND),
        SkillStatus::Broken => ("BROKEN", BAD),
    }
}

fn installation_style(kind: &InstallationKind) -> (&'static str, Color) {
    match kind {
        InstallationKind::ManagedSymlink => ("managed link", GOOD),
        InstallationKind::Physical => ("physical copy", WARN),
        InstallationKind::ForeignSymlink => ("foreign link", BAD),
        InstallationKind::BrokenSymlink => ("broken link", BAD),
    }
}

fn centered(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width.saturating_sub(2));
    let height = height.min(area.height.saturating_sub(2));
    Rect::new(
        area.x + area.width.saturating_sub(width) / 2,
        area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TargetConfig;
    use ratatui::backend::TestBackend;
    use std::{collections::BTreeMap, fs};

    fn fixture() -> (tempfile::TempDir, Config) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("root");
        let target = temp.path().join("agent");
        fs::create_dir_all(root.join("rust-cli")).unwrap();
        fs::write(root.join("rust-cli/SKILL.md"), "body").unwrap();
        fs::create_dir_all(&target).unwrap();
        let config = Config {
            root,
            targets: BTreeMap::from([(
                "claude".to_string(),
                TargetConfig {
                    path: target,
                    enabled: true,
                },
            )]),
            relative_links: false,
            ignore: vec![],
        };
        (temp, config)
    }

    #[test]
    fn dashboard_renders_disabled_skills_and_help() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, false).unwrap();
        let backend = TestBackend::new(110, 32);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("skill-issue"));
        assert!(screen.contains("rust-cli"));
        assert!(screen.contains("DISABLED"));
        assert!(screen.contains("1 managed"), "{screen}");
        assert!(screen.contains("0 broken"));
        assert!(screen.contains("0 conflicts"));
        assert!(screen.contains("0 pending"));
        assert_eq!(screen.matches("managed").count(), 1, "{screen}");
        assert_eq!(screen.matches("conflicts").count(), 1, "{screen}");
        assert!(screen.contains("1 skills  ·  1 agents"), "{screen}");
        assert!(screen.contains("[ SKILLS ]"), "{screen}");
        assert!(screen.contains("│"), "{screen}");

        app.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(terminal.backend().to_string().contains("Keyboard guide"));
    }

    #[test]
    fn filter_handles_matches_and_empty_results() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, true).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE))
            .unwrap();
        for character in "missing".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(character), KeyModifiers::NONE))
                .unwrap();
        }
        assert!(app.filtered_skills().is_empty());
        assert_eq!(app.skill_state.selected(), None);
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.filtered_skills().len(), 1);
        assert_eq!(app.skill_state.selected(), Some(0));
    }

    #[test]
    fn dashboard_renders_safely_in_a_small_terminal() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, false).unwrap();
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(!terminal.backend().to_string().trim().is_empty());
    }

    #[test]
    fn narrow_dashboard_keeps_the_summary_visible() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, false).unwrap();
        let backend = TestBackend::new(50, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("managed"), "{screen}");
        assert!(screen.contains("broken"), "{screen}");
        assert!(screen.contains("conflicts"), "{screen}");
        assert!(screen.contains("pending"), "{screen}");
    }

    #[test]
    fn dry_run_confirmation_never_changes_files() {
        let (_temp, config) = fixture();
        let destination = config.targets["claude"].path.join("rust-cli");
        let mut app = App::new(config, true, false).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(!destination.exists());
        assert!(app.notice.contains("Dry run"));
    }

    #[test]
    fn live_refresh_picks_up_filesystem_changes() {
        let (_temp, config) = fixture();
        let mut app = App::new(config.clone(), false, false).unwrap();
        fs::create_dir_all(config.root.join("new-skill")).unwrap();
        fs::write(config.root.join("new-skill/SKILL.md"), "new").unwrap();

        app.live_refresh();

        assert!(app.result.groups.contains_key("new-skill"));
    }

    #[test]
    fn cancelled_confirmation_keeps_the_skill_unchanged() {
        let (_temp, config) = fixture();
        let destination = config.targets["claude"].path.join("rust-cli");
        let mut app = App::new(config, false, false).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(!destination.exists());
        assert!(matches!(app.mode, Mode::Browse));
    }

    #[test]
    fn x_opens_the_delete_confirmation_for_the_selected_skill() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, false).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();

        assert!(matches!(app.mode, Mode::ConfirmDelete(_)));
    }

    #[test]
    fn tab_navigation_renders_agent_and_health_views() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, false).unwrap();
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.view, View::Agents);
        app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        terminal.draw(|frame| app.draw(frame)).unwrap();
        let screen = terminal.backend().to_string();
        assert!(screen.contains("Agent details"));
        assert!(screen.contains("[ AGENTS ]"), "{screen}");

        app.handle_key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.view, View::Health);
        terminal.draw(|frame| app.draw(frame)).unwrap();
        assert!(terminal.backend().to_string().contains("Canonical health"));

        app.handle_key(KeyEvent::new(KeyCode::BackTab, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.view, View::Agents);
    }

    #[test]
    fn health_gauge_label_is_centered_and_has_consistent_contrast() {
        let (_temp, config) = fixture();
        let mut app = App::new(config, false, false).unwrap();
        app.view = View::Health;
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| app.draw(frame)).unwrap();

        let buffer = terminal.backend().buffer();
        let label = " 1/1 fully managed ";
        let (label_y, label_x) = (0..buffer.area.height)
            .find_map(|y| {
                let row: String = (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol())
                    .collect::<Vec<_>>()
                    .concat();
                row.find(label)
                    .map(|byte| (y, row[..byte].chars().count() as u16))
            })
            .expect("gauge label should be visible");
        let label_center = label_x + label.len() as u16 / 2;
        assert!(
            label_center.abs_diff(buffer.area.width / 2) <= 1,
            "label starts at {label_x} and centers at {label_center} in width {}",
            buffer.area.width
        );
        for x in label_x..label_x + label.len() as u16 {
            let cell = &buffer[(x, label_y)];
            assert_eq!(cell.fg, Color::White);
            assert_eq!(cell.bg, Color::Rgb(15, 23, 42));
        }
    }

    #[test]
    fn uncollected_skill_toggle_shows_safe_guidance() {
        let (temp, mut config) = fixture();
        fs::remove_dir_all(config.root.join("rust-cli")).unwrap();
        let physical = temp.path().join("agent/uncollected");
        fs::create_dir_all(&physical).unwrap();
        fs::write(physical.join("SKILL.md"), "body").unwrap();
        config.targets.get_mut("claude").unwrap().path = temp.path().join("agent");
        let mut app = App::new(config, false, false).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(app.mode, Mode::Notice));
        assert!(app.notice.contains("si setup"));
    }

    #[cfg(unix)]
    #[test]
    fn confirmed_toggle_enables_then_disables_a_skill() {
        let (_temp, config) = fixture();
        let destination = config.targets["claude"].path.join("rust-cli");
        let mut app = App::new(config, false, false).unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(app.mode, Mode::Confirm(_)));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(destination.is_symlink());
        app.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE))
            .unwrap();
        app.handle_key(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE))
            .unwrap();
        assert!(!destination.exists());
    }

    #[cfg(unix)]
    #[test]
    fn confirmed_sync_reconciles_and_refreshes_the_dashboard() {
        let (_temp, config) = fixture();
        let destination = config.targets["claude"].path.join("rust-cli");
        let mut app = App::new(config, false, false).unwrap();

        app.handle_key(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE))
            .unwrap();
        assert!(matches!(app.mode, Mode::ConfirmSync(_)));
        app.handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();

        assert!(destination.is_symlink());
        assert_eq!(app.result.groups["rust-cli"].status(), SkillStatus::Managed);
        assert!(app.notice.contains("Sync complete"));
    }
}
