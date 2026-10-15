use guinea::slint::Routable;

#[derive(Clone, PartialEq, Debug, Routable)]
pub enum Route {
    Home,
    Host { name: String, port: u16 },
    Processes(ProcessesRoute),
}

#[derive(Clone, PartialEq, Debug, Routable)]
pub enum ProcessesRoute {
    List,
    One { pid: u32 },
}
