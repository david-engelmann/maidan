package maidan

import (
	"bytes"
	"strings"
	"testing"
)

func TestDecodeModelJSONHappy(t *testing.T) {
	var ws Workspace
	err := DecodeModelJSON(strings.NewReader(`{"id":"ws_1","name":"n","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}`), &ws)
	if err != nil {
		t.Fatal(err)
	}
	if ws.ID != "ws_1" || ws.Name != "n" {
		t.Fatalf("unexpected workspace: %+v", ws)
	}
}

func TestDecodeModelJSONRejectsOversize(t *testing.T) {
	body := []byte(`{"id":"` + strings.Repeat("a", MaxModelJSONBytes) + `"}`)
	var ws Workspace
	err := DecodeModelJSON(bytes.NewReader(body), &ws)
	if err == nil || !strings.Contains(err.Error(), "exceeds") {
		t.Fatalf("expected oversize error, got %v", err)
	}
}

func TestDecodeModelJSONRejectsTrailingJunk(t *testing.T) {
	var ws Workspace
	err := DecodeModelJSON(strings.NewReader(`{"id":"ws_1","name":"n","created_at":"2026-01-01T00:00:00Z","updated_at":"2026-01-01T00:00:00Z"}{"x":1}`), &ws)
	if err == nil {
		t.Fatal("expected trailing junk error")
	}
}

func TestDecodeModelJSONNilReader(t *testing.T) {
	var ws Workspace
	if err := DecodeModelJSON(nil, &ws); err == nil {
		t.Fatal("expected nil reader error")
	}
}
