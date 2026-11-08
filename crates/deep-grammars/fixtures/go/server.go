package server

import (
	"context"
	"net/http"
)

type Server struct {
	addr string
	log  func(string)
}

func (s *Server) Start(ctx context.Context) error {
	done := make(chan error, 1)
	go func() {
		done <- http.ListenAndServe(s.addr, nil)
	}()
	select {
	case err := <-done:
		return err
	case <-ctx.Done():
		s.log("shutdown")
		return nil
	}
}

func (s *Server) Serve(handler http.Handler) {
	switch s.addr {
	case "":
		s.log("no address")
	case "localhost:0":
		s.log("ephemeral")
	default:
		http.ListenAndServe(s.addr, handler)
	}
}
