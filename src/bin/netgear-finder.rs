use clap::Parser;
use std::fmt::Write;
use std::io::prelude::*;
use std::net::{SocketAddr, SocketAddrV4};
use std::sync::Arc;
use strum::{EnumIter, IntoEnumIterator};
use tracing::{debug, info, trace, warn};

#[derive(Parser)]
struct Opt {
    /// The colon separated style mac-address of the this PC
    #[arg(short, long)]
    mac_address: MacAddress,

    /// A wait in milliseconds after sending datagrams
    #[arg(short, long, default_value = "0")]
    delay: u64,

    /// Stop after sending count datagrams
    #[arg(short, long, default_value = "1")]
    count: usize,

    /// Interactive mode for firewall
    #[arg(short, long)]
    interactive: bool,
}

#[derive(Clone)]
struct MacAddress([u8; 6]);

impl std::str::FromStr for MacAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut mac_address_array = [0u8; 6];

        for (index, entry) in s.split(':').enumerate() {
            if mac_address_array.len() <= index {
                anyhow::bail!("unexpected format: {}", s)
            }
            mac_address_array[index] = u8::from_str_radix(entry, 16)?;
        }

        Ok(MacAddress(mac_address_array))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenv::dotenv().ok();
    tracing_subscriber::fmt::init();

    let opt: Opt = Opt::parse();
    let mac_address = opt.mac_address;

    let create_server = |tx: tokio::sync::mpsc::Sender<(SocketAddr, DeviceInfo)>,
                         socket_rx: Arc<tokio::net::UdpSocket>| {
        tokio::spawn(async move {
            loop {
                info!("start recv");
                let mut ret_buf = vec![0u8; 4096];
                let (n, from_address) = match socket_rx.recv_from(&mut ret_buf).await {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(?e, "failed to retrieve data");
                        continue;
                    }
                };
                debug!(%n, %from_address);
                let device_info = match decode_datagram(&ret_buf[..n]) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!(?e, "failed to decode datagram");
                        continue;
                    }
                };
                info!(ip = %from_address.ip(), ?device_info, "decoded");
                tx.send((from_address, device_info)).await.ok();
            }
        })
    };

    let (result_tx, mut result_rx) = tokio::sync::mpsc::channel(3);

    let socket_tx_v2 =
        Arc::new(tokio::net::UdpSocket::bind(SocketAddrV4::new([0, 0, 0, 0].into(), 63321)).await?);
    socket_tx_v2.set_broadcast(true)?;
    let handle_v2 = create_server(result_tx.clone(), socket_tx_v2.clone());

    let socket_tx_v1 =
        Arc::new(tokio::net::UdpSocket::bind(SocketAddrV4::new([0, 0, 0, 0].into(), 63323)).await?);
    socket_tx_v1.set_broadcast(true)?;
    let handle_v1 = create_server(result_tx, socket_tx_v1.clone());

    let target_v2 = SocketAddrV4::new([255, 255, 255, 255].into(), 63322);
    let target_v1 = SocketAddrV4::new([255, 255, 255, 255].into(), 63324);
    let send_datagram_ad_v2 = create_message_ad_v2(&mac_address);
    let send_datagram_udp_v2 = create_message_udp_v2(&mac_address);
    let send_datagram_ad_v1 = create_message_ad(&mac_address);
    let send_datagram_udp_v1 = create_message_udp(&mac_address);

    socket_tx_v2
        .send_to(&send_datagram_ad_v2, &target_v2)
        .await
        .ok();
    socket_tx_v2
        .send_to(&send_datagram_udp_v2, &target_v2)
        .await
        .ok();

    socket_tx_v1
        .send_to(&send_datagram_ad_v1, &target_v1)
        .await
        .ok();
    socket_tx_v1
        .send_to(&send_datagram_udp_v1, &target_v1)
        .await
        .ok();

    if opt.interactive {
        eprint!("Press return to continue: ");

        std::io::stdin().read_line(&mut String::new())?;
    } else {
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
    }

    for _ in 0..(opt.count - 1) {
        socket_tx_v2
            .send_to(&send_datagram_ad_v2, &target_v2)
            .await
            .ok();
        socket_tx_v1
            .send_to(&send_datagram_ad_v1, &target_v2)
            .await
            .ok();
        tokio::time::sleep(tokio::time::Duration::from_millis(opt.delay)).await;
    }

    handle_v2.abort();
    handle_v1.abort();

    let mut result_mac_addresses = std::collections::HashSet::new();

    while let Some((from_address, data)) = result_rx.recv().await {
        if result_mac_addresses.contains(&data.mac_address) {
            continue;
        }

        result_mac_addresses.insert(data.mac_address.to_owned());
        println!("{} {:?}", from_address, data);
    }

    Ok(())
}

fn create_message_ad(mac_address: &MacAddress) -> Vec<u8> {
    vec![
        0x01, // Version
        0x01, // Command
        0x00, // Status
        0x00, // Reserve
        0x00,
        0x00, // Failure TLV
        0x00,
        0x00, // Reserve
        mac_address.0[0],
        mac_address.0[1],
        mac_address.0[2],
        mac_address.0[3],
        mac_address.0[4],
        mac_address.0[5], // Manager ID - Source MAC Address
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00, // Agent Id - Dest MAC Address (Broadcast)
        0x00,
        0x00,
        0x00,
        0x0b, // Sequence Number
        0x4e,
        0x53,
        0x44,
        0x50, // Protocol Signature NSDP
        0x00,
        0x00,
        0x00,
        0x00, // Reserve
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x02,
        0x00,
        0x00,
        0x00,
        0x03,
        0x00,
        0x00,
        0x00,
        0x04,
        0x00,
        0x00,
        0x00,
        0x05,
        0x00,
        0x00,
        0x00,
        0x06,
        0x00,
        0x00,
        0x00,
        0x07,
        0x00,
        0x00,
        0x00,
        0x08,
        0x00,
        0x00,
        0x00,
        0x0b,
        0x00,
        0x00,
        0x00,
        0x0c,
        0x00,
        0x00,
        0x00,
        0x0d,
        0x00,
        0x00,
        0x00,
        0x0e,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x74, // Body
        0x00,
        0x00,
        0x00,
        0xff,
        0xff,
        0x00,
        0x00, // Marker
    ]
}

fn create_message_ad_v2(mac_address: &MacAddress) -> Vec<u8> {
    vec![
        0x01, // Version
        0x01, // Command
        0x00, // Status
        0x00, // Reserve
        0x00,
        0x00, // Failure TLV
        0x00,
        0x00, // Reserve
        mac_address.0[0],
        mac_address.0[1],
        mac_address.0[2],
        mac_address.0[3],
        mac_address.0[4],
        mac_address.0[5], // Manager ID - Source MAC Address
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00, // Agent Id - Dest MAC Address (Broadcast)
        0x00,
        0x00,
        0x01,
        0x02, // Sequence Number
        0x4e,
        0x53,
        0x44,
        0x50, // Protocol Signature NSDP
        0x00,
        0x00,
        0x00,
        0x00, // Reserve
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x03,
        0x00,
        0x00,
        0x00,
        0x04,
        0x00,
        0x00,
        0x00,
        0x06,
        0x00,
        0x00,
        0x00,
        0x07,
        0x00,
        0x00,
        0x00,
        0x08,
        0x00,
        0x00,
        0x00,
        0x0b,
        0x00,
        0x00,
        0x00,
        0x0c,
        0x00,
        0x00,
        0x00,
        0x0d,
        0x00,
        0x00,
        0x00,
        0x0e,
        0x00,
        0x00,
        0x00,
        0x0f,
        0x00,
        0x00,
        0x00,
        0x14,
        0x00,
        0x00,
        0x78, // Body
        0x00,
        0x00,
        0x00,
        0x74, // Body
        0x00,
        0x00,
        0x00,
        0xff,
        0xff,
        0x00,
        0x00, // Marker
    ]
}

fn create_message_udp(mac_address: &MacAddress) -> Vec<u8> {
    vec![
        0x01, // Version
        0x01, // Command
        0x00, // Status
        0x00, // Reserve
        0x00,
        0x00, // Failure TLV
        0x00,
        0x00, // Reserve
        mac_address.0[0],
        mac_address.0[1],
        mac_address.0[2],
        mac_address.0[3],
        mac_address.0[4],
        mac_address.0[5], // Manager ID - Source MAC Address
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00, // Agent Id - Dest MAC Address (Broadcast)
        0x00,
        0x00,
        0x00,
        0x00, // Sequence Number
        0x4e,
        0x53,
        0x44,
        0x50, // Protocol Signature NSDP
        0x00,
        0x00,
        0x00,
        0x00, // Reserve
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x02,
        0x00,
        0x00,
        0x00,
        0x03,
        0x00,
        0x00,
        0x00,
        0x04,
        0x00,
        0x00,
        0x00,
        0x05,
        0x00,
        0x00,
        0x00,
        0x06,
        0x00,
        0x00,
        0x00,
        0x07,
        0x00,
        0x00,
        0x00,
        0x08,
        0x00,
        0x00,
        0x00,
        0x0b,
        0x00,
        0x00,
        0x00,
        0x0c,
        0x00,
        0x00,
        0x00,
        0x0d,
        0x00,
        0x00,
        0x00,
        0x0e,
        0x00,
        0x00,
        0x00,
        0x0f,
        0x00,
        0x00, // Body
        0xff,
        0xff,
        0x00,
        0x00, // Marker
    ]
}

fn create_message_udp_v2(mac_address: &MacAddress) -> Vec<u8> {
    vec![
        0x01, // Version
        0x01, // Command
        0x00, // Status
        0x00, // Reserve
        0x00,
        0x00, // Failure TLV
        0x00,
        0x00, // Reserve
        mac_address.0[0],
        mac_address.0[1],
        mac_address.0[2],
        mac_address.0[3],
        mac_address.0[4],
        mac_address.0[5], // Manager ID - Source MAC Address
        0x00,
        0x00,
        0x00,
        0x00,
        0x00,
        0x00, // Agent Id - Dest MAC Address (Broadcast)
        0x00,
        0x00,
        0x01,
        0x0d, // Sequence Number
        0x4e,
        0x53,
        0x44,
        0x50, // Protocol Signature NSDP
        0x00,
        0x00,
        0x00,
        0x00, // Reserve
        0x00,
        0x01,
        0x00,
        0x00,
        0x00,
        0x03,
        0x00,
        0x00,
        0x00,
        0x04,
        0x00,
        0x00,
        0x00,
        0x06,
        0x00,
        0x00,
        0x00,
        0x07,
        0x00,
        0x00,
        0x00,
        0x08,
        0x00,
        0x00,
        0x00,
        0x0b,
        0x00,
        0x00,
        0x00,
        0x0c,
        0x00,
        0x00,
        0x00,
        0x0d,
        0x00,
        0x00,
        0x00,
        0x0e,
        0x00,
        0x00,
        0x00,
        0x0f,
        0x00,
        0x00,
        0x00,
        0x14,
        0x00,
        0x00,
        0x78, // Body
        0x00,
        0x00,
        0x00,
        0x74, // Body
        0x00,
        0x00,
        0x00,
        0xff,
        0xff,
        0x00,
        0x00, // Marker
    ]
}

#[derive(Debug, EnumIter)]
enum DatagramTag {
    Model,
    Name,
    MacAddress,
    Network,
    Firmware,
    Firmware2,
    SerialNumber,
}

impl DatagramTag {
    fn tag(&self) -> [u8; 2] {
        match self {
            Self::Model => [0x00, 0x01],
            Self::Name => [0x00, 0x03],
            Self::MacAddress => [0x00, 0x04],
            Self::Network => [0x00, 0x08],
            Self::Firmware => [0x00, 0x0d],
            Self::Firmware2 => [0x00, 0x0e],
            Self::SerialNumber => [0x78, 0x00],
        }
    }
}

impl std::convert::TryFrom<[u8; 2]> for DatagramTag {
    type Error = anyhow::Error;

    fn try_from(value: [u8; 2]) -> Result<Self, Self::Error> {
        for entry in DatagramTag::iter() {
            if entry.tag() == value {
                return Ok(entry);
            }
        }

        anyhow::bail!("not found: {:?}", value)
    }
}

#[derive(Debug, Default)]
struct DeviceInfo {
    model_name: String,
    name: String,
    mac_address: String,
    network: String,
    firmware: String,
    firmware2: String,
    serial_number: String,
}

fn decode_datagram(datagram: &[u8]) -> anyhow::Result<DeviceInfo> {
    if datagram.len() < 32 + 2 {
        anyhow::bail!("too short datagram")
    }

    let mut datagram = &datagram[32..];
    let mut buf = [0u8; 2];
    let mut section_it = DatagramTag::iter();
    let mut section: DatagramTag = section_it.next().unwrap();
    let mut info: DeviceInfo = Default::default();

    loop {
        debug!(parse = ?section);

        let n = datagram.read(&mut buf)?;
        if n != 2 {
            info!("n != 2");
            break;
        }

        if buf != section.tag() {
            let n = datagram.read(&mut buf)?;

            if n != 2 {
                anyhow::bail!("n != 2; todo")
            }

            let len = (buf[0] as usize) << 8 | buf[1] as usize;

            if 0 < len {
                trace!(%len, "consume");
                if datagram.len() <= len {
                    anyhow::bail!(
                        "slice index starts at {} but ends at {}",
                        len,
                        datagram.len()
                    );
                }
                datagram.consume(len)
            }
            trace!("continue");
            continue;
        }

        let n = datagram.read(&mut buf)?;

        if n != 2 {
            anyhow::bail!("unexpected payload")
        }

        let len = (buf[0] as usize) << 8 | buf[1] as usize;

        if len == 0 {
            trace!("len == 0");
            match section_it.next() {
                Some(data) => section = data,
                None => break,
            }
            continue;
        }

        let mut payload = vec![0u8; len];
        datagram.read_exact(&mut payload)?;

        match section {
            DatagramTag::Model => {
                write!(&mut info.model_name, "{}", std::str::from_utf8(&payload)?)?
            }
            DatagramTag::Name => write!(&mut info.name, "{}", std::str::from_utf8(&payload)?)?,
            DatagramTag::MacAddress => {
                for entry in payload.iter() {
                    if !info.mac_address.is_empty() {
                        info.mac_address.push(':');
                    }
                    write!(&mut info.mac_address, "{:02X}", entry)?;
                }
            }
            DatagramTag::Network => {
                for entry in payload.iter() {
                    if !info.network.is_empty() {
                        info.network.push('.');
                    }
                    write!(&mut info.network, "{}", entry)?;
                }
            }
            DatagramTag::Firmware => {
                write!(&mut info.firmware, "{}", std::str::from_utf8(&payload)?)?
            }
            DatagramTag::Firmware2 => {
                write!(&mut info.firmware2, "{}", std::str::from_utf8(&payload)?)?
            }
            DatagramTag::SerialNumber => write!(
                &mut info.serial_number,
                "{}",
                std::str::from_utf8(&payload)?
            )?,
        }

        match section_it.next() {
            Some(data) => section = data,
            None => break,
        }
    }

    Ok(info)
}

#[cfg(test)]
mod tests {
    use std::io::prelude::*;

    #[test]
    fn read_u16() {
        let data = [1u8, 2];
        let mut buf = [0u8; 2];
        let n = (&data[..]).read(&mut buf).unwrap();

        assert_eq!(n, 2);
        assert_eq!((buf[0] as u16) << 8 | buf[1] as u16, 258);
    }

    #[test]
    fn vec_size() {
        let size = 4;
        let mut data = vec![0u8; size as usize];
        assert_eq!(data.len(), 4);

        let source = [0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10];
        (&source[..]).read(&mut data).unwrap();

        assert_eq!(data.len(), 4);
        assert_eq!(&data[..], [0, 1, 2, 3]);
    }

    #[test]
    fn rang() {
        assert_eq!(
            (0..(-1))
                .map(|data| format!("{}", data))
                .collect::<Vec<_>>()
                .join(","),
            ""
        );
    }
}
