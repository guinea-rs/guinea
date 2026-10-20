fn main() -> anyhow::Result<()> {
    guinea::ratatui::run(|app| {
        todo_core::startup(app, "tui")?;
        Ok(todo_tui::window())
    })
}
