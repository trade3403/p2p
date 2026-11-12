package main

// Minimal IPv4 STUN Binding responder for opt-in network diagnostics.
// It does not authenticate a device, forward media, allocate TURN relays,
// implement ICE or promise a traversable NAT mapping.
// RFC 8489 Binding success + XOR-MAPPED-ADDRESS.
import (
    "encoding/binary"
    "errors"
    "net"
    "time"
)

const stunMagicCookie uint32 = 0x2112A442
const maxSTUNRequest = 256

func bindingResponse(request []byte, peer *net.UDPAddr) []byte {
    if len(request) < 20 || len(request) > maxSTUNRequest || peer == nil {
        return nil
    }
    if binary.BigEndian.Uint16(request[:2]) != 0x0001 ||
        binary.BigEndian.Uint32(request[4:8]) != stunMagicCookie {
        return nil
    }
    payloadSize := int(binary.BigEndian.Uint16(request[2:4]))
    if payloadSize%4 != 0 || payloadSize+20 != len(request) {
        return nil
    }
    ipv4 := peer.IP.To4()
    if ipv4 == nil || peer.Port < 1 || peer.Port > 65535 {
        return nil
    }

    out := make([]byte, 32)
    binary.BigEndian.PutUint16(out[0:2], 0x0101)
    binary.BigEndian.PutUint16(out[2:4], 12)
    binary.BigEndian.PutUint32(out[4:8], stunMagicCookie)
    copy(out[8:20], request[8:20])
    binary.BigEndian.PutUint16(out[20:22], 0x0020) // XOR-MAPPED-ADDRESS
    binary.BigEndian.PutUint16(out[22:24], 8)
    out[24], out[25] = 0, 0x01 // IPv4 address family
    binary.BigEndian.PutUint16(out[26:28], uint16(peer.Port)^0x2112)
    cookie := []byte{0x21, 0x12, 0xA4, 0x42}
    for i := 0; i < 4; i++ {
        out[28+i] = ipv4[i] ^ cookie[i]
    }
    return out
}

func serveSTUN(socket *net.UDPConn, stop <-chan struct{}) error {
    if socket == nil {
        return errors.New("nil STUN socket")
    }
    packet := make([]byte, maxSTUNRequest+1)
    for {
        if err := socket.SetReadDeadline(time.Now().Add(250*time.Millisecond)); err != nil {
            return err
        }
        n, addr, err := socket.ReadFromUDP(packet)
        if err != nil {
            var timeout net.Error
            if errors.As(err, &timeout) && timeout.Timeout() {
                select {
                case <-stop:
                    return nil
                default:
                    continue
                }
            }
            select {
            case <-stop:
                return nil
            default:
                return err
            }
        }
        response := bindingResponse(packet[:n], addr)
        if response != nil {
            _, _ = socket.WriteToUDP(response, addr)
        }
    }
}
