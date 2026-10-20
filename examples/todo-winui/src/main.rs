fn main() -> anyhow::Result<()> {
    guinea::winui::run(|app| {
        todo_core::startup(app, "winui")?;
        Ok(todo_winui::window())
    })
}
