package generic

import "fmt"

type Number interface {
	~int | ~int64 | ~float64
}

func Sum[T Number](values []T) T {
	var total T
	for _, v := range values {
		total += v
	}
	return total
}

func Describe(value interface{}) string {
	switch v := value.(type) {
	case nil:
		return "nil"
	case int:
		return fmt.Sprintf("int:%d", v)
	case string:
		return "string:" + v
	default:
		return "other"
	}
}

func Zero[T any]() (zero T, ok bool) {
	return zero, false
}
