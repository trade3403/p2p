package main

import (
	"crypto/subtle"
	"errors"
	"flag"
	"log"
	"net/http"
	"net"
	"os"
	"regexp"
	"strings"
	"sync"
	"time"

	"github.com/gorilla/websocket"
)

var roomRE = regexp.MustCompile(`^[a-f0-9]{64}$`)

const maxPacketSize = 3 << 20

type peer struct {
	conn *websocket.Conn
	send chan []byte
	done chan struct{}
	once sync.Once
}

func (p *peer) stop() { p.once.Do(func() { close(p.done); p.conn.Close() }) }

type room struct{ host, viewer *peer }
type hub struct {
	mu    sync.Mutex
	rooms map[string]*room
	token string
}

func newHub(token string) *hub { return &hub{rooms: make(map[string]*room), token: token} }
func (h *hub) claim(id, role string, p *peer) error {
	h.mu.Lock()
	defer h.mu.Unlock()
	r := h.rooms[id]
	if r == nil {
		r = &room{}
		h.rooms[id] = r
	}
	if role == "host" {
		if r.host != nil {
			return errors.New("host already connected")
		}
		r.host = p
	} else {
		if r.viewer != nil {
			return errors.New("viewer already connected")
		}
		r.viewer = p
	}
	return nil
}
func (h *hub) leave(id, role string, p *peer) {
	h.mu.Lock()
	defer h.mu.Unlock()
	r := h.rooms[id]
	if r == nil {
		return
	}
	if role == "host" && r.host == p {
		r.host = nil
	}
	if role == "viewer" && r.viewer == p {
		r.viewer = nil
	}
	if r.host == nil && r.viewer == nil {
		delete(h.rooms, id)
	}
}
func (h *hub) receiver(id, role string, p *peer) *peer {
	h.mu.Lock()
	defer h.mu.Unlock()
	r := h.rooms[id]
	if r == nil {
		return nil
	}
	if role == "host" {
		return r.viewer
	}
	return r.host
}
func (h *hub) serve(w http.ResponseWriter, r *http.Request) {
	id, role := r.URL.Query().Get("room"), r.URL.Query().Get("role")
	if !roomRE.MatchString(id) || (role != "host" && role != "viewer") {
		http.Error(w, "invalid room or role", 400)
		return
	}
	if subtle.ConstantTimeCompare([]byte(strings.TrimPrefix(r.Header.Get("Authorization"), "Bearer ")), []byte(h.token)) != 1 || !strings.HasPrefix(r.Header.Get("Authorization"), "Bearer ") {
		http.Error(w, "unauthorized", 401)
		return
	}
	// Only Caddy should be reachable publicly. Local direct access is for health checks.
	if r.Header.Get("X-Forwarded-Proto") != "https" && r.TLS == nil && !isLocal(r.RemoteAddr) {
		http.Error(w, "TLS required", 426)
		return
	}
	upgrader := websocket.Upgrader{ReadBufferSize: 4096, WriteBufferSize: 4096, CheckOrigin: func(r *http.Request) bool { return true }}
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	p := &peer{conn: conn, send: make(chan []byte, 12), done: make(chan struct{})}
	if err = h.claim(id, role, p); err != nil {
		conn.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(websocket.ClosePolicyViolation, "already connected"), time.Now().Add(time.Second))
		p.stop()
		return
	}
	defer func() { h.leave(id, role, p); p.stop() }()
	go func() {
		ping := time.NewTicker(25 * time.Second)
		defer ping.Stop()
		for {
			select {
			case pkt := <-p.send:
				p.conn.SetWriteDeadline(time.Now().Add(8 * time.Second))
				if p.conn.WriteMessage(websocket.BinaryMessage, pkt) != nil {
					p.stop()
					return
				}
			case <-ping.C:
				p.conn.SetWriteDeadline(time.Now().Add(8 * time.Second))
				if p.conn.WriteMessage(websocket.PingMessage, nil) != nil {
					p.stop()
					return
				}
			case <-p.done:
				return
			}
		}
	}()
	conn.SetReadLimit(maxPacketSize)
	conn.SetReadDeadline(time.Now().Add(75 * time.Second))
	conn.SetPongHandler(func(_ string) error { conn.SetReadDeadline(time.Now().Add(75 * time.Second)); return nil })
	for {
		msgType, data, err := conn.ReadMessage()
		if err != nil {
			return
		}
		if msgType != websocket.BinaryMessage || len(data) == 0 {
			continue
		}
		target := h.receiver(id, role, p)
		if target == nil {
			continue
		}
		select {
		case target.send <- data:
		default: /* drop frames when network backpressures */
		}
	}
}
func isLocal(addr string) bool {
	return strings.HasPrefix(addr, "127.0.0.1:") || strings.HasPrefix(addr, "[::1]:")
}
func main() {
	listen := flag.String("listen", ":8080", "listen address (use internal Docker network)")
	flag.Parse()
	token := os.Getenv("RELAY_ACCESS_TOKEN")
	if len(token) < 32 {
		log.Fatal("RELAY_ACCESS_TOKEN must be at least 32 characters")
	}
    if os.Getenv("STUN_ENABLED") == "true" {
        address := os.Getenv("STUN_LISTEN_ADDR")
        if address == "" { address = ":3478" }
        endpoint, err := net.ResolveUDPAddr("udp4", address)
        if err != nil { log.Fatalf("invalid STUN bind address: %v", err) }
        server, err := net.ListenUDP("udp4", endpoint)
        if err != nil { log.Fatalf("STUN UDP bind failed: %v", err) }
        log.Printf("optional STUN Binding listener enabled on %s (UDP)", address)
        go func() {
            if err := serveSTUN(server, make(chan struct{})); err != nil {
                log.Printf("STUN service stopped: %v", err)
            }
        }()
    }
	h := newHub(token)
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", func(w http.ResponseWriter, _ *http.Request) { w.WriteHeader(200); w.Write([]byte("ok\n")) })
	mux.HandleFunc("/relay", h.serve)
	srv := &http.Server{Addr: *listen, Handler: mux, ReadHeaderTimeout: 7 * time.Second}
	log.Printf("relay listening on %s (terminate TLS at Caddy)", *listen)
	log.Fatal(srv.ListenAndServe())
}
