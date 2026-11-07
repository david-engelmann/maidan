package maidan

import "testing"

func TestWsHostHeaderOmitsDefaultPorts(t *testing.T) {
	if got := wsHostHeader("maidan.world", "443", true); got != "maidan.world" {
		t.Fatalf("wss default port Host = %q", got)
	}
	if got := wsHostHeader("maidan.world", "80", false); got != "maidan.world" {
		t.Fatalf("ws default port Host = %q", got)
	}
	if got := wsHostHeader("maidan.world", "8443", true); got != "maidan.world:8443" {
		t.Fatalf("wss non-default Host = %q", got)
	}
	if got := wsHostHeader("127.0.0.1", "8080", false); got != "127.0.0.1:8080" {
		t.Fatalf("ws non-default Host = %q", got)
	}
}
