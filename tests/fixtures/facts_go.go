package shop

import (
	"fmt"
	str "strings"
)

type Repo struct {
	name string
}

func NewRepo(name string) *Repo {
	return &Repo{name: name}
}

func (r *Repo) Find(id int) (string, error) {
	trimmed := str.TrimSpace(r.name)
	fmt.Println(trimmed)
	return trimmed, nil
}
