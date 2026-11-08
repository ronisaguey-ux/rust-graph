package models

import "time"

type UserID int64

type User struct {
	ID        UserID    `json:"id"`
	Name      string    `json:"name"`
	Email     string    `json:"email,omitempty"`
	CreatedAt time.Time `json:"created_at"`
	Tags      []string  `json:"tags"`
}

type Repository interface {
	Find(id UserID) (*User, error)
	Save(u *User) error
	List(limit int) ([]*User, error)
}

type Alias = User

type Audit struct {
	UserID UserID
	Action string
	Meta   map[string]interface{}
}

func (a *Audit) Describe() string {
	if a.Meta == nil {
		return a.Action
	}
	return a.Action + " by " + string(a.UserID)
}
