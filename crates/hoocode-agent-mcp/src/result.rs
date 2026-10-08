//! Tool results as plain content for the model (decision M2 in `docs/design/mcp.md`).

use rmcp::model::{CallToolResult, ContentBlock, ResourceContents};

/// One piece of a tool result. Text and images are passed through; other
/// content is turned into a short text note.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolContent {
    Text(String),
    /// Base64 data with its MIME type, as the server sent it.
    Image {
        mime_type: String,
        data: String,
    },
}

/// The result of one `tools/call`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolOutput {
    pub content: Vec<ToolContent>,
    /// The tool ran and reported a failure (`isError`). Not a transport error.
    pub is_error: bool,
}

impl ToolOutput {
    /// An error result with one text note, for example for an oversized response.
    pub fn error_text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ToolContent::Text(text.into())],
            is_error: true,
        }
    }

    /// The number of payload bytes (text and base64 data), used for the size cap.
    pub fn payload_bytes(&self) -> usize {
        self.content
            .iter()
            .map(|c| match c {
                ToolContent::Text(t) => t.len(),
                ToolContent::Image { data, .. } => data.len(),
            })
            .sum()
    }
}

/// Converts an rmcp result. Audio and binary resources become a text note,
/// since the model cannot read them here.
pub(crate) fn from_call_result(result: CallToolResult) -> ToolOutput {
    let content = result
        .content
        .into_iter()
        .map(|block| match block {
            ContentBlock::Text(text) => ToolContent::Text(text.text),
            ContentBlock::Image(image) => ToolContent::Image {
                mime_type: image.mime_type,
                data: image.data,
            },
            ContentBlock::Audio(audio) => {
                ToolContent::Text(format!("[audio content, {}]", audio.mime_type))
            }
            ContentBlock::Resource(embedded) => match embedded.resource {
                ResourceContents::TextResourceContents { text, .. } => ToolContent::Text(text),
                ResourceContents::BlobResourceContents { uri, mime_type, .. } => {
                    ToolContent::Text(format!(
                        "[binary resource {uri}{}]",
                        mime_type.map(|m| format!(", {m}")).unwrap_or_default()
                    ))
                }
                _ => ToolContent::Text("[unsupported resource]".to_owned()),
            },
            ContentBlock::ResourceLink(link) => {
                ToolContent::Text(format!("[resource link {}]", link.uri))
            }
            _ => ToolContent::Text("[unsupported content]".to_owned()),
        })
        .collect();
    ToolOutput {
        content,
        is_error: result.is_error.unwrap_or(false),
    }
}
