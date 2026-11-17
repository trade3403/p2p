package main

import (
	"net/http"
	"net/http/httptest"
	"strings"
	"testing"
)

func TestRelayRejectsMissingBearerToken(t *testing.T) {
	h := newHub(strings.Repeat("f", 64))
	req := httptest.NewRequest(http.MethodGet, "/relay?room="+strings.Repeat("a", 64)+"&role=viewer", nil)
	response := httptest.NewRecorder()
	h.serve(response, req)
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("got %d, want 401", response.Code)
	}
}

func TestRelayRejectsWrongBearerToken(t *testing.T) {
	h := newHub(strings.Repeat("f", 64))
	req := httptest.NewRequest(http.MethodGet, "/relay?room="+strings.Repeat("a", 64)+"&role=viewer", nil)
	req.Header.Set("Authorization", "Bearer "+strings.Repeat("e", 64))
	req.RemoteAddr = "127.0.0.1:12345"
	response := httptest.NewRecorder()
	h.serve(response, req)
	if response.Code != http.StatusUnauthorized {
		t.Fatalf("got %d, want 401", response.Code)
	}
}

func TestRelayRejectsMalformedRoom(t *testing.T) {
	h := newHub(strings.Repeat("f", 64))
	req := httptest.NewRequest(http.MethodGet, "/relay?room=../invalid&role=viewer", nil)
	req.Header.Set("Authorization", "Bearer "+strings.Repeat("f", 64))
	response := httptest.NewRecorder()
	h.serve(response, req)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("got %d, want 400", response.Code)
	}
}

func TestRelayRejectsInvalidRole(t *testing.T) {
	h := newHub(strings.Repeat("f", 64))
	req := httptest.NewRequest(http.MethodGet, "/relay?room="+strings.Repeat("a", 64)+"&role=admin", nil)
	req.Header.Set("Authorization", "Bearer "+strings.Repeat("f", 64))
	response := httptest.NewRecorder()
	h.serve(response, req)
	if response.Code != http.StatusBadRequest {
		t.Fatalf("got %d, want 400", response.Code)
	}
}

func TestRelayRejectsSpoofedForwardedProtoFromPublicIP(t *testing.T) {
	h := newHub(strings.Repeat("f", 64))
	req := httptest.NewRequest(http.MethodGet, "/relay?room="+strings.Repeat("a", 64)+"&role=viewer", nil)
	req.Header.Set("Authorization", "Bearer "+strings.Repeat("f", 64))
	req.Header.Set("X-Forwarded-Proto", "https")
	req.RemoteAddr = "203.0.113.10:54321"
	response := httptest.NewRecorder()
	h.serve(response, req)
	if response.Code != http.StatusUpgradeRequired {
		t.Fatalf("got %d, want 426", response.Code)
	}
}

func TestRelayRejectsBrowserOrigin(t *testing.T) {
	h := newHub(strings.Repeat("f", 64))
	req := httptest.NewRequest(http.MethodGet, "/relay?room="+strings.Repeat("a", 64)+"&role=viewer", nil)
	req.Header.Set("Authorization", "Bearer "+strings.Repeat("f", 64))
	req.Header.Set("Origin", "https://evil.example")
	req.RemoteAddr = "127.0.0.1:12345"
	response := httptest.NewRecorder()
	h.serve(response, req)
	if response.Code != http.StatusForbidden {
		t.Fatalf("got %d, want 403", response.Code)
	}
}

func TestHealthzIsMinimal(t *testing.T) {
	req := httptest.NewRequest(http.MethodGet, "/healthz", nil)
	response := httptest.NewRecorder()
	healthz(response, req)
	if response.Code != http.StatusOK {
		t.Fatalf("got %d", response.Code)
	}
	body := response.Body.String()
	if body != "ok\n" {
		t.Fatalf("body %q", body)
	}
	lower := strings.ToLower(body)
	for _, needle := range []string{"token", "room", "bearer", "secret", "viewer", "host"} {
		if strings.Contains(lower, needle) {
			t.Fatalf("healthz leaked %q", needle)
		}
	}
}

func TestAuthorizedConstantTimeAPI(t *testing.T) {
	token := strings.Repeat("a", 64)
	if !authorized("Bearer "+token, token) {
		t.Fatal("valid token rejected")
	}
	if authorized("Bearer "+strings.Repeat("b", 64), token) {
		t.Fatal("wrong token accepted")
	}
	if authorized(token, token) {
		t.Fatal("missing Bearer prefix accepted")
	}
}

func TestTLSGatePrivateProxy(t *testing.T) {
	req := httptest.NewRequest(http.MethodGet, "/relay", nil)
	req.RemoteAddr = "172.18.0.2:443"
	req.Header.Set("X-Forwarded-Proto", "https")
	if !tlsGate(req) {
		t.Fatal("private proxy with https forward should pass")
	}
	req.Header.Set("X-Forwarded-Proto", "http")
	if tlsGate(req) {
		t.Fatal("http forward should fail")
	}
}
