//! RFC 8489 STUN Binding diagnostic.
//! A one-shot UDP socket discovers only an observed mapping. The socket
//! closes immediately and CANNOT be reused as a P2P candidate for ICE.
//! No session keys, relay bearer tokens or desktop data are transmitted.
use anyhow::{anyhow, bail, Context, Result};
use rand::RngCore;
use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, ToSocketAddrs, UdpSocket},
    time::{Duration, Instant},
};

const COOKIE: [u8; 4] = [0x21, 0x12, 0xA4, 0x42];
const UDP_PORT: u16 = 3478;
const STUN_MAX: usize = 512;

pub fn request() -> [u8; 20] {
    let mut output = [0u8; 20];
    output[1] = 1; // Binding Request
    output[4..8].copy_from_slice(&COOKIE);
    rand::rngs::OsRng.fill_bytes(&mut output[8..20]);
    output
}

pub fn parse_response(response: &[u8], req: &[u8;20]) -> Result<SocketAddrV4> {
    if response.len() < 32 || response.len() > STUN_MAX {
        bail!("invalid STUN response length");
    }
    if response[0..2] != [0x01, 0x01]
        || response[4..8] != COOKIE
        || response[8..20] != req[8..20]
    {
        bail!("STUN transaction mismatch or invalid response type");
    }
    let body_size = u16::from_be_bytes([response[2], response[3]]) as usize;
    if body_size + 20 != response.len() || body_size % 4 != 0 {
        bail!("invalid STUN response framing");
    }
    let mut offset = 20;
    while offset + 4 <= response.len() {
        let attr_kind = u16::from_be_bytes([response[offset], response[offset+1]]);
        let attr_len = u16::from_be_bytes([response[offset+2],response[offset+3]]) as usize;
        let start = offset + 4;
        let end = start.checked_add(attr_len).context("STUN attribute overflow")?;
        if end > response.len() { bail!("invalid STUN attribute"); }
        if attr_kind == 0x0020 && attr_len == 8 && response[start+1] == 1 {
            let port = u16::from_be_bytes([response[start+2],response[start+3]]) ^ 0x2112;
            let addr = Ipv4Addr::new(
                response[start+4] ^ COOKIE[0],
                response[start+5] ^ COOKIE[1],
                response[start+6] ^ COOKIE[2],
                response[start+7] ^ COOKIE[3],
            );
            if port == 0 { bail!("invalid observed STUN port"); }
            return Ok(SocketAddrV4::new(addr,port));
        }
        offset = start + ((attr_len+3)&!3);
    }
    bail!("no IPv4 XOR-MAPPED-ADDRESS in STUN response")
}

pub fn probe_from_relay(relay_url: &str) -> Result<(SocketAddrV4, Duration)> {
    let url = url::Url::parse(relay_url.trim()).context("invalid relay URL")?;
    if !matches!(url.scheme(), "https" | "wss")
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
    {
        bail!("a clean HTTPS/WSS relay address is required");
    }
    let host = url.host_str().ok_or_else(|| anyhow!("missing relay hostname"))?;
    let server = (host,UDP_PORT).to_socket_addrs()
        .context("STUN DNS lookup failed")?
        .find(|addr| matches!(addr,SocketAddr::V4(_)))
        .context("STUN server has no IPv4 address")?;
    probe(server)
}

pub fn probe(server: SocketAddr) -> Result<(SocketAddrV4,Duration)> {
    if !server.is_ipv4() { bail!("only IPv4 STUN diagnostics supported"); }
    let sock = UdpSocket::bind("0.0.0.0:0").context("cannot bind local UDP")?;
    sock.set_read_timeout(Some(Duration::from_secs(2)))?;
    sock.set_write_timeout(Some(Duration::from_secs(2)))?;
    sock.connect(server)?;
    let req=request();
    let start=Instant::now();
    sock.send(&req).context("STUN request could not be sent")?;
    let mut bytes=[0u8;STUN_MAX];
    let n=sock.recv(&mut bytes).context("STUN response timeout or UDP blocked")?;
    let address=parse_response(&bytes[..n],&req)?;
    Ok((address,start.elapsed()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(req:&[u8;20],ip:[u8;4],port:u16) -> [u8;32] {
        let mut out=[0u8;32];
        out[0]=1;out[1]=1;out[3]=12;
        out[4..8].copy_from_slice(&COOKIE);
        out[8..20].copy_from_slice(&req[8..20]);
        out[21]=0x20;out[23]=8;out[25]=1;
        out[26..28].copy_from_slice(&(port ^ 0x2112).to_be_bytes());
        for i in 0..4 {out[28+i]=ip[i]^COOKIE[i];}
        out
    }
    #[test]
    fn parses_public_mapping_and_rejects_transaction_mismatch() {
        let req=request();
        let mut message=response(&req,[198,51,100,26],43781);
        assert_eq!(parse_response(&message,&req).unwrap(),
            SocketAddrV4::new(Ipv4Addr::new(198,51,100,26),43781));
        message[10]^=1;
        assert!(parse_response(&message,&req).is_err());
        assert!(parse_response(&message[..25],&req).is_err());
    }
    #[test]
    fn rejects_missing_xor_attribute_and_zero_port() {
        let req=request();
        let mut message=response(&req,[198,51,100,26],100);
        message[20]=0;message[21]=1;
        assert!(parse_response(&message,&req).is_err());
        message=response(&req,[198,51,100,26],0);
        assert!(parse_response(&message,&req).is_err());
    }
    #[test]
    fn actual_udp_loopback_receives_transaction_response() {
        let server=UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr=server.local_addr().unwrap();
        let handler=std::thread::spawn(move || {
            let mut data=[0u8;100];
            let (count,peer)=server.recv_from(&mut data).unwrap();
            assert_eq!(count,20);
            let mut req=[0u8;20];
            req.copy_from_slice(&data[..count]);
            server.send_to(&response(&req,[127,0,0,1],peer.port()),peer).unwrap();
        });
        let (observed,_rtt)=probe(addr).unwrap();
        assert_eq!(*observed.ip(),Ipv4Addr::LOCALHOST);
        handler.join().unwrap();
    }
}
