use std::fmt::{self, Display};

#[derive(Debug)]
pub(crate) struct Diagnostic {
    pub(crate) message: String,
    pub(crate) related: Vec<Self>,
}

impl Diagnostic {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            related: Vec::new(),
        }
    }

    pub(crate) fn with_related(mut self, related: Vec<Self>) -> Self {
        self.related = related;
        self
    }

    pub(crate) fn render(&self) -> String {
        let mut rendered = String::new();
        miette::GraphicalReportHandler::new_themed(miette::GraphicalTheme::unicode_nocolor())
            .with_show_related_as_nested(true)
            .with_wrap_lines(false)
            .render_report(&mut rendered, self)
            .expect("rendering a diagnostic into a String");
        rendered
    }
}

impl Display for Diagnostic {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

impl miette::Diagnostic for Diagnostic {
    fn related<'a>(&'a self) -> Option<Box<dyn Iterator<Item = &'a dyn miette::Diagnostic> + 'a>> {
        (!self.related.is_empty()).then(|| {
            Box::new(
                self.related
                    .iter()
                    .map(|related| related as &dyn miette::Diagnostic),
            ) as Box<dyn Iterator<Item = &dyn miette::Diagnostic>>
        })
    }
}
