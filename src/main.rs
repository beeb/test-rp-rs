#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]
#![feature(async_fn_in_trait)]
#![allow(incomplete_features)]

use core::convert::Infallible;

use cyw43::NetDriver;
use defmt::*;
use embassy_executor::Spawner;
use embassy_net::{
    dns::{self, DnsQueryType},
    tcp::client::{TcpClient, TcpClientState},
    Config, Stack, StackResources,
};
use embassy_rp::peripherals::{PIN_23, PIN_24, PIN_25, PIN_29};
use embassy_rp::{
    clocks::RoscRng,
    gpio::{Flex, Level, Output},
};
use embassy_time::{Duration, Timer};
use embedded_hal_1::spi::ErrorType;
use embedded_hal_async::spi::{ExclusiveDevice, SpiBusFlush, SpiBusRead, SpiBusWrite};
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
        ExclusiveDevice<MySpi, Output<'static, PIN_25>>,
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
    let clk = Output::new(p.PIN_29, Level::Low);
    let mut dio = Flex::new(p.PIN_24);
    dio.set_low();
    dio.set_as_output();

    let bus = MySpi { clk, dio };
    let spi = ExclusiveDevice::new(bus, cs);

    let state = singleton!(cyw43::State::new());
    let (net_device, mut control, runner) = cyw43::new(state, pwr, spi, fw).await;

    spawner.spawn(wifi_task(runner)).unwrap();

    control.init(clm).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    control
        .join_wpa2(env!("WIFI_NETWORK"), env!("WIFI_PASSWORD"))
        .await;

    let config = Config::Dhcp(Default::default());

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

    unwrap!(spawner.spawn(net_task(stack)));

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

struct MySpi {
    /// SPI clock
    clk: Output<'static, PIN_29>,

    /// 4 signals, all in one!!
    /// - SPI MISO
    /// - SPI MOSI
    /// - IRQ
    /// - strap to set to gSPI mode on boot.
    dio: Flex<'static, PIN_24>,
}

impl ErrorType for MySpi {
    type Error = Infallible;
}

impl SpiBusFlush for MySpi {
    async fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl SpiBusRead<u32> for MySpi {
    async fn read(&mut self, words: &mut [u32]) -> Result<(), Self::Error> {
        self.dio.set_as_input();
        for word in words {
            let mut w = 0;
            for _ in 0..32 {
                w <<= 1;

                // rising edge, sample data
                if self.dio.is_high() {
                    w |= 0x01;
                }
                self.clk.set_high();

                // falling edge
                self.clk.set_low();
            }
            *word = w
        }

        Ok(())
    }
}

impl SpiBusWrite<u32> for MySpi {
    async fn write(&mut self, words: &[u32]) -> Result<(), Self::Error> {
        self.dio.set_as_output();
        for word in words {
            let mut word = *word;
            for _ in 0..32 {
                // falling edge, setup data
                self.clk.set_low();
                if word & 0x8000_0000 == 0 {
                    self.dio.set_low();
                } else {
                    self.dio.set_high();
                }

                // rising edge
                self.clk.set_high();

                word <<= 1;
            }
        }
        self.clk.set_low();

        self.dio.set_as_input();
        Ok(())
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
