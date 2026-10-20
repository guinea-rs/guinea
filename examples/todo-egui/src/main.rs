fn main() -> anyhow::Result<()> {
    guinea::eframe::run(|app| {
        todo_core::startup(app, "egui")?;
        Ok(todo_egui::window())
    })
}
