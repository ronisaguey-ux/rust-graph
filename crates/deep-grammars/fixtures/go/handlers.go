package handlers

import (
	"net/http"
	"strconv"
)

type Handler struct {
	repo map[int]string
}

func (h *Handler) Get(w http.ResponseWriter, r *http.Request) {
	id, err := strconv.Atoi(r.URL.Query().Get("id"))
	if err != nil {
		http.Error(w, "bad id", http.StatusBadRequest)
		return
	}
	value, ok := h.repo[id]
	if !ok {
		w.WriteHeader(http.StatusNotFound)
		return
	}
	w.Write([]byte(value))
}

func (h *Handler) All() []string {
	out := []string{}
	for _, v := range h.repo {
		out = append(out, v)
	}
	return out
}

func Routes() []string {
	return []string{"GET /", "POST /items"}
}
