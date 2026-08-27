// Package maidan is the Go client for Maidan.
// 0.0.1 is a name reservation; the API is not stable.
package maidan

type Client struct {
	BaseURL string
	Token   string
}

func New(baseURL, token string) *Client {
	return &Client{BaseURL: baseURL, Token: token}
}
