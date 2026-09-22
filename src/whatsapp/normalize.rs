use whatsapp_rust::bot::MessageContext;
use whatsapp_rust::prelude::MessageExt;

use crate::incoming::{CommandContent, IncomingCommand};

pub async fn normalize(
    context: &MessageContext,
) -> Result<IncomingCommand, crate::error::AgentError> {
    let message = context.message.get_base_message();
    let content = if let Some(text) = message.text_content() {
        CommandContent::Text(text.to_owned())
    } else if let Some(audio) = message.audio_message.as_option() {
        let data = context
            .client
            .download(audio)
            .await
            .map_err(|error| crate::error::AgentError::MediaDownload(error.to_string()))?;
        CommandContent::Voice {
            mime_type: audio.mimetype.clone(),
            data,
        }
    } else {
        CommandContent::UnsupportedMedia
    };

    Ok(IncomingCommand {
        message_id: context.info.id.as_str().to_owned(),
        chat_id: context.info.source.chat.to_string(),
        sender_id: context.info.source.sender.to_string(),
        timestamp: context.info.timestamp,
        content,
    })
}
