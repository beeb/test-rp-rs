use defmt::*;
use embedded_nal_async::{heapless::String, Dns, TcpConnect};
use reqwless::{
    client::HttpClient,
    request::{Method, RequestBuilder},
};

pub mod commands;

pub async fn notify_start<'a, T, D>(
    client: &'a mut HttpClient<'a, T, D>,
) -> &'a mut HttpClient<'a, T, D>
where
    T: TcpConnect + 'a,
    D: Dns + 'a,
{
    let url = concat!(
        "https://discord.com/api/v10/channels/",
        env!("DISCORD_CHANNEL_ID"),
        "/messages"
    );
    debug!("URL: {}", url);

    let mut content: String<2000> = String::new();
    content.push_str("{\"content\": \"Hello World!\"}").unwrap();
    debug!("Content: {}", content);
    let mut req_rx_buf = [0; 4096];
    let headers = [
        ("Authorization", concat!("Bot ", env!("DISCORD_BOT_TOKEN"))),
        (
            "User-Agent",
            concat!(
                "DiscordBot (",
                env!("CARGO_PKG_HOMEPAGE"),
                ", ",
                env!("CARGO_PKG_VERSION"),
                ")"
            ),
        ),
    ]
    .as_slice();

    match client.request(Method::POST, url).await {
        Err(e) => {
            error!("Can't create POST request: {:?}", e);
        }
        Ok(req) => {
            if let Err(e) = req
                .body(content.as_bytes())
                .content_type(reqwless::headers::ContentType::ApplicationJson)
                .headers(headers)
                .send(&mut req_rx_buf)
                .await
            {
                error!("HTTP POST Error: {:?}", e);
            }
        }
    }

    client
}
