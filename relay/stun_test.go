package main

import (
    "bytes"
    "encoding/binary"
    "net"
    "testing"
    "time"
)

func validRequest() []byte {
    b := make([]byte, 20)
    binary.BigEndian.PutUint16(b[0:2], 1)
    binary.BigEndian.PutUint32(b[4:8], stunMagicCookie)
    copy(b[8:20], []byte("test-id-1234"))
    return b
}

func TestSTUNBindingIPv4XORMappedAddress(t *testing.T) {
    peer := &net.UDPAddr{IP: net.IPv4(203,0,113,27), Port: 45678}
    req := validRequest()
    got := bindingResponse(req, peer)
    if len(got)!=32 {
        t.Fatalf("response length=%d", len(got))
    }
    if binary.BigEndian.Uint16(got[0:2])!=0x0101 ||
        binary.BigEndian.Uint16(got[2:4])!=12 ||
        binary.BigEndian.Uint16(got[20:22])!=0x0020 ||
        got[25]!=1 ||
        !bytes.Equal(got[8:20], req[8:20]) {
        t.Fatal("invalid STUN binding-success response")
    }
    if binary.BigEndian.Uint16(got[26:28])^0x2112 != 45678 {
        t.Fatal("XOR encoded port differs")
    }
    cookie := []byte{0x21,0x12,0xa4,0x42}
    for i,v := range []byte{203,0,113,27} {
        if got[28+i]^cookie[i]!=v {t.Fatal("XOR encoded IPv4 differs")}
    }
}

func TestSTUNRejectsMalformedRequests(t *testing.T) {
    addr := &net.UDPAddr{IP:net.IPv4(127,0,0,1), Port:20000}
    req := validRequest()
    for _, candidate := range [][]byte{
        {}, req[:8],
        append(bytes.Clone(req), 0),
        func() []byte {x:=bytes.Clone(req);x[1]=0x03;return x}(),
        func() []byte {x:=bytes.Clone(req);x[4]=0;return x}(),
        func() []byte {x:=bytes.Clone(req);x[3]=4;return x}(),
        make([]byte,257),
    } {
        if bindingResponse(candidate,addr)!=nil {
            t.Fatalf("accepted malformed candidate %v",candidate)
        }
    }
    if bindingResponse(req,&net.UDPAddr{IP:net.ParseIP("::1"),Port:10000})!=nil {
        t.Fatal("unexpected IPv6 response from IPv4-only listener")
    }
}

func TestSTUNUDPActualLoopback(t *testing.T) {
    server,err := net.ListenUDP("udp4",&net.UDPAddr{IP:net.IPv4(127,0,0,1),Port:0})
    if err!=nil {t.Fatal(err)}
    defer server.Close()
    stop := make(chan struct{})
    done := make(chan error,1)
    go func(){done<-serveSTUN(server,stop)}()
    client,err := net.DialUDP("udp4",nil,server.LocalAddr().(*net.UDPAddr))
    if err!=nil {t.Fatal(err)}
    defer client.Close()
    client.SetDeadline(time.Now().Add(3*time.Second))
    req := validRequest()
    if _,err=client.Write(req);err!=nil {t.Fatal(err)}
    b:=make([]byte,128)
    n,err:=client.Read(b)
    if err!=nil {t.Fatal(err)}
    if n!=32 || !bytes.Equal(b[8:20],req[8:20]) {t.Fatal("loopback STUN response mismatch")}
    close(stop)
    if err = <-done;err!=nil {t.Fatal(err)}
}
