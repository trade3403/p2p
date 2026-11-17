package main

import (
	"crypto/subtle"
	"errors"
	"flag"
	"log"
	"net"
	"net/http"
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
	mu       sync.Mutex
	rooms    map[string]*room
	token    string
	limiter  *ipLimiter
}

func newHub(token string) *hub {
	return &hub{rooms: make(map[string]*room), token: token, limiter: newIPLimiter(30, time.Minute)}
}

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

func authorized(header, token string) bool {
	const prefix = "Bearer "
	if !strings.HasPrefix(header, prefix) {
		return false
	}
	got := []byte(strings.TrimPrefix(header, prefix))
	want := []byte(token)
	// subtle.ConstantTimeCompare returns 0 when lengths differ.
	return subtle.ConstantTimeCompare(got, want) == 1
}

// tlsGate ensures public clients only arrive via TLS-terminated Caddy.
// X-Forwarded-Proto is trusted only from loopback or private Docker/network peers,
// so a mis-published :8080 cannot be bypassed by spoofing the header from the Internet.
func tlsGate(r *http.Request) bool {
	if r.TLS != nil {
		return true
	}
	host, _, err := net.SplitHostPort(r.RemoteAddr)
	if err != nil {
		host = r.RemoteAddr
	}
	ip := net.ParseIP(host)
	if ip == nil {
		return false
	}
	if ip.IsLoopback() {
		return true
	}
	if isPrivateIP(ip) && r.Header.Get("X-Forwarded-Proto") == "https" {
		return true
	}
	return false
}

func isPrivateIP(ip net.IP) bool {
	if ip.IsLoopback() || ip.IsLinkLocalUnicast() || ip.IsLinkLocalMulticast() {
		return true
	}
	privateCIDRs := []string{
		"10.0.0.0/8",
		"172.16.0.0/12",
		"192.168.0.0/16",
		"fc00::/7",
	}
	for _, cidr := range privateCIDRs {
		_, network, err := net.ParseCIDR(cidr)
		if err != nil {
			continue
		}
		if network.Contains(ip) {
			return true
		}
	}
	return false
}

func originAllowed(r *http.Request) bool {
	origin := r.Header.Get("Origin")
	if origin == "" {
		// Native Windows/Android clients typically omit Origin.
		return true
	}
	// Browser clients are not a supported control plane; reject foreign Origins.
	return false
}

func (h *hub) serve(w http.ResponseWriter, r *http.Request) {
	id, role := r.URL.Query().Get("room"), r.URL.Query().Get("role")
	if !roomRE.MatchString(id) || (role != "host" && role != "viewer") {
		http.Error(w, "invalid room or role", http.StatusBadRequest)
		return
	}
	if !authorized(r.Header.Get("Authorization"), h.token) {
		http.Error(w, "unauthorized", http.StatusUnauthorized)
		return
	}
	if !tlsGate(r) {
		http.Error(w, "TLS required", http.StatusUpgradeRequired)
		return
	}
	if !originAllowed(r) {
		http.Error(w, "origin not allowed", http.StatusForbidden)
		return
	}
	if host, _, err := net.SplitHostPort(r.RemoteAddr); err == nil {
		if !h.limiter.allow(host) {
			http.Error(w, "rate limited", http.StatusTooManyRequests)
			return
		}
	}
	upgrader := websocket.Upgrader{
		ReadBufferSize:  4096,
		WriteBufferSize: 4096,
		CheckOrigin: func(r *http.Request) bool {
			return originAllowed(r)
		},
	}
	conn, err := upgrader.Upgrade(w, r, nil)
	if err != nil {
		return
	}
	p := &peer{conn: conn, send: make(chan []byte, 12), done: make(chan struct{})}
	if err = h.claim(id, role, p); err != nil {
		_ = conn.WriteControl(websocket.CloseMessage, websocket.FormatCloseMessage(websocket.ClosePolicyViolation, "already connected"), time.Now().Add(time.Second))
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
				_ = p.conn.SetWriteDeadline(time.Now().Add(8 * time.Second))
				if p.conn.WriteMessage(websocket.BinaryMessage, pkt) != nil {
					p.stop()
					return
				}
			case <-ping.C:
				_ = p.conn.SetWriteDeadline(time.Now().Add(8 * time.Second))
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
	_ = conn.SetReadDeadline(time.Now().Add(75 * time.Second))
	conn.SetPongHandler(func(_ string) error {
		return conn.SetReadDeadline(time.Now().Add(75 * time.Second))
	})
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

// ipLimiter is a coarse per-IP connection attempt budget for the personal relay.
type ipLimiter struct {
	mu       sync.Mutex
	limit    int
	window   time.Duration
	attempts map[string][]time.Time
}

func newIPLimiter(limit int, window time.Duration) *ipLimiter {
	return &ipLimiter{limit: limit, window: window, attempts: make(map[string][]time.Time)}
}

func (l *ipLimiter) allow(ip string) bool {
	l.mu.Lock()
	defer l.mu.Unlock()
	now := time.Now()
	cut := now.Add(-l.window)
	kept := l.attempts[ip][:0]
	for _, t := range l.attempts[ip] {
		if t.After(cut) {
			kept = append(kept, t)
		}
	}
	if len(kept) >= l.limit {
		l.attempts[ip] = kept
		return false
	}
	l.attempts[ip] = append(kept, now)
	return true
}

func healthz(w http.ResponseWriter, _ *http.Request) {
	// Intentionally minimal: no token, rooms, or peer metadata.
	w.Header().Set("Content-Type", "text/plain; charset=utf-8")
	w.WriteHeader(http.StatusOK)
	_, _ = w.Write([]byte("ok\n"))
}

func main() {
	listen := flag.String("listen", ":8080", "listen address (use internal Docker network)")
	flag.Parse()
	token := os.Getenv("RELAY_ACCESS_TOKEN")
	if len(token) < 32 {
		log.Fatal("RELAY_ACCESS_TOKEN must be at least 32 characters")
	}
	h := newHub(token)
	mux := http.NewServeMux()
	mux.HandleFunc("/healthz", healthz)
	mux.HandleFunc("/relay", h.serve)
	srv := &http.Server{Addr: *listen, Handler: mux, ReadHeaderTimeout: 7 * time.Second}
	log.Printf("relay listening on %s (terminate TLS at Caddy)", *listen)
	log.Fatal(srv.ListenAndServe())
}
