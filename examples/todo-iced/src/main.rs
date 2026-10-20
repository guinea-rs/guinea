fn main() -> anyhow::Result<()> {
    guinea::iced::run(|app| {
        todo_core::startup(app, "iced")?;
        Ok(todo_iced::window())
    })
}
