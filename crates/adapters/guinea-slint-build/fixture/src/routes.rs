#[derive(Clone, PartialEq, Debug, guinea::slint::Route)]
pub enum Route {
    Home,
    Host { name: String, port: u16 },
    Processes(ProcessesRoute),
}

#[derive(Clone, PartialEq, Debug, guinea::slint::Route)]
pub enum ProcessesRoute {
    List,
    One { pid: u32 },
}
