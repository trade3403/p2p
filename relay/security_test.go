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
