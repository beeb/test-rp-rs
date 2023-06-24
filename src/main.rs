#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

use cyw43::NetDriver;
use cyw43_pio::PioSpi;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::{
    dns::{self, DnsQueryType},
    tcp::client::{TcpClient, TcpClientState},
    Config, DhcpConfig, Stack, StackResources,
};
use embassy_rp::{
    clocks::RoscRng,
    gpio::{Level, Output},
    peripherals::{DMA_CH0, PIN_23, PIN_25, PIO0},
    pio::Pio,
};
use embassy_time::{Duration, Timer};
use embedded_nal_async::{heapless::String, AddrType, Dns, IpAddr, Ipv4Addr};
use rand_core::RngCore;
use reqwless::{
    client::{HttpClient, TlsConfig, TlsVerify},
    request::{Method, RequestBuilder},
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

macro_rules! singleton {
    ($val:expr) => {{
        type T = impl Sized;
        static STATIC_CELL: StaticCell<T> = StaticCell::new();
        STATIC_CELL.init_with(move || $val)
    }};
}

#[embassy_executor::task]
async fn wifi_task(
    runner: cyw43::Runner<
        'static,
        Output<'static, PIN_23>,
        PioSpi<'static, PIN_25, PIO0, 0, DMA_CH0>,
    >,
) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(stack: &'static Stack<cyw43::NetDriver<'static>>) -> ! {
    stack.run().await
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Hello World!");

    let p = embassy_rp::init(Default::default());

    // Include the WiFi firmware and Country Locale Matrix (CLM) blobs.
    let fw = include_bytes!("../firmware/43439A0.bin");
    let clm = include_bytes!("../firmware/43439A0_clm.bin");

    let pwr = Output::new(p.PIN_23, Level::Low);
    let cs = Output::new(p.PIN_25, Level::High);

    let mut pio = Pio::new(p.PIO0);

    let spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        pio.irq0,
        cs,
        p.PIN_24,
        p.PIN_29,
        p.DMA_CH0,
    );

    let state = singleton!(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;
    spawner.spawn(wifi_task(runner)).unwrap();

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let dhcp_config = DhcpConfig::default();
    let config = Config::dhcpv4(dhcp_config);

    // Generate random seed
    let mut rng = RoscRng {};
    let seed = rng.next_u64();

    // Init network stack
    let stack = &*singleton!(Stack::new(
        net_device,
        config,
        singleton!(StackResources::<3>::new()),
        seed
    ));

    spawner.spawn(net_task(stack)).unwrap();

    control
        .join_wpa2(env!("WIFI_NETWORK"), env!("WIFI_PASSWORD"))
        .await
        .unwrap();

    info!("Network stack initialized");

    static STATE: TcpClientState<1, 1024, 1024> = TcpClientState::new();
    debug!("State initialized");
    let client = TcpClient::new(stack, &STATE);
    debug!("TCP Client initialized");
    let mut tls_read_buffer = [0; 16384];
    let mut tls_write_buffer = [0; 16384];
    let tls_seed = rng.next_u64();
    let tls_config = TlsConfig::new(
        tls_seed,
        &mut tls_read_buffer,
        &mut tls_write_buffer,
        TlsVerify::None,
    );
    debug!("TLS Config initialized");
    //let dns = StaticDnsResolver {};
    let dns = DnsResolver { stack };
    debug!("DNS Resolver initialized");
    let mut client = HttpClient::new_with_tls(&client, &dns, tls_config);
    debug!("HTTP Client initialized");

    let url = concat!(
        "https://discord.com/api/channels/",
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
    debug!("Headers: {:?}", headers);

    loop {
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
        Timer::after(Duration::from_secs(10)).await;
    }
}

struct DnsResolver {
    stack: &'static Stack<NetDriver<'static>>,
}

impl Dns for DnsResolver {
    type Error = dns::Error;

    async fn get_host_by_name(
        &self,
        host: &str,
        addr_type: AddrType,
    ) -> Result<IpAddr, Self::Error> {
        let dns_query_type = match addr_type {
            AddrType::IPv4 => DnsQueryType::A,
            AddrType::IPv6 => DnsQueryType::Aaaa,
            AddrType::Either => DnsQueryType::A,
        };
        let res = self.stack.dns_query(host, dns_query_type).await?;
        let res = res.first().ok_or(dns::Error::Failed)?;
        let addr = res.as_bytes();
        debug!(
            "Resolved {} to {}.{}.{}.{}",
            host, addr[0], addr[1], addr[2], addr[3]
        );
        let addr = IpAddr::V4(Ipv4Addr::new(addr[0], addr[1], addr[2], addr[3]));
        Ok(addr)
    }

    async fn get_host_by_address(&self, _addr: IpAddr) -> Result<String<256>, Self::Error> {
        Ok(String::new())
    }
}
