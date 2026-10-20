fn main() -> anyhow::Result<()> {
    guinea::slint::run(|app| {
        todo_core::startup(app, "slint")?;
        todo_slint::window()
    })
}
