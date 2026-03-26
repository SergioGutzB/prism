use crate::review::models::{CommentSource, GeneratedComment, ReviewDraft, Severity};

/// Builder for creating manual review comments interactively.
pub struct ManualCommentBuilder {
    pub file_path: Option<String>,
    pub line: Option<u32>,
    pub body: String,
    pub severity: Severity,
}

impl ManualCommentBuilder {
    pub fn new() -> Self {
        Self {
            file_path: None,
            line: None,
            body: String::new(),
            severity: Severity::Suggestion,
        }
    }

    pub fn file_path(mut self, path: impl Into<String>) -> Self {
        self.file_path = Some(path.into());
        self
    }

    pub fn line(mut self, line: u32) -> Self {
        self.line = Some(line);
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> Self {
        self.body = body.into();
        self
    }

    pub fn severity(mut self, severity: Severity) -> Self {
        self.severity = severity;
        self
    }

    pub fn build(self) -> GeneratedComment {
        GeneratedComment::new(
            CommentSource::Manual,
            self.body,
            self.severity,
            self.file_path,
            self.line,
        )
    }
}

impl Default for ManualCommentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Add a manual comment to a draft.
pub fn add_manual_comment(
    draft: &mut ReviewDraft,
    body: String,
    severity: Severity,
    file_path: Option<String>,
    line: Option<u32>,
) {
    let comment = ManualCommentBuilder::new()
        .body(body)
        .severity(severity);

    let comment = if let Some(path) = file_path {
        comment.file_path(path)
    } else {
        comment
    };

    let comment = if let Some(l) = line {
        comment.line(l)
    } else {
        comment
    };

    draft.add_comment(comment.build());
}
