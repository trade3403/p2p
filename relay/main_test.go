package main

import "testing"

func TestRoomValidation(t *testing.T) {
	if !roomRE.MatchString("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa") {
		t.Fatal("valid room rejected")
	}
	for _, x := range []string{"", "../../test", "zzzz", "abcd"} {
		if roomRE.MatchString(x) {
			t.Fatalf("accepted %q", x)
		}
	}
}
func TestClaimSingleViewer(t *testing.T) {
	h := newHub("a-really-long-development-only-secret-key")
	p := &peer{}
	if err := h.claim("room", "viewer", p); err != nil {
		t.Fatal(err)
	}
	if err := h.claim("room", "viewer", &peer{}); err == nil {
		t.Fatal("accepted second viewer")
	}
	h.leave("room", "viewer", p)
	if len(h.rooms) != 0 {
		t.Fatal("room leak")
	}
}
