package main

import (
    "bytes"
    "net/http"
    "net/http/httptest"
    "strings"
    "testing"
    "time"

    "github.com/gorilla/websocket"
)

// The relay must forward encrypted V0.9 signaling without attempting to
// decrypt/interpret the message. Both peers still require a bearer token.
func TestV09EncryptedCandidateSignalingRelaysAsOpaqueBytes(t *testing.T) {
    h := newHub(strings.Repeat("f", 64))
    server := httptest.NewServer(http.HandlerFunc(h.serve))
    defer server.Close()

    room := strings.Repeat("a", 64)
    endpoint := "ws" + strings.TrimPrefix(server.URL, "http") + "/relay?room=" + room
    headers := http.Header{"Authorization": []string{"Bearer " + strings.Repeat("f",64)}}
    host, _, err := websocket.DefaultDialer.Dial(endpoint + "&role=host", headers)
    if err != nil { t.Fatalf("host connect: %v", err) }
    defer host.Close()
    viewer, _, err := websocket.DefaultDialer.Dial(endpoint + "&role=viewer", headers)
    if err != nil { t.Fatalf("viewer connect: %v", err) }
    defer viewer.Close()

    payload := append([]byte{10}, bytes.Repeat([]byte{0x3a}, 45)...)
    if err = viewer.WriteMessage(websocket.BinaryMessage, payload); err != nil {
        t.Fatal(err)
    }
    host.SetReadDeadline(time.Now().Add(3*time.Second))
    kind, got, err := host.ReadMessage()
    if err != nil { t.Fatalf("host did not receive UDP candidate signal: %v", err) }
    if kind != websocket.BinaryMessage || !bytes.Equal(got, payload) {
        t.Fatal("relay modified or dropped encrypted UDP signaling")
    }

    response := append([]byte{10}, bytes.Repeat([]byte{0x7d}, 48)...)
    if err = host.WriteMessage(websocket.BinaryMessage, response); err != nil {
        t.Fatal(err)
    }
    viewer.SetReadDeadline(time.Now().Add(3*time.Second))
    kind, got, err = viewer.ReadMessage()
    if err != nil { t.Fatalf("viewer did not receive encrypted candidate signal: %v", err) }
    if kind != websocket.BinaryMessage || !bytes.Equal(got, response) {
        t.Fatal("relay modified or dropped reverse UDP signaling")
    }
}
