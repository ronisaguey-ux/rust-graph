package pipeline

import "strings"

const (
	StageInput = iota
	StageMap
	StageOutput
)

var separators = []string{",", ";", "|"}

type Stage interface {
	Process(line string) (string, error)
}

type Transform func(string) string

func Split(line string) []string {
	fields := strings.FieldsFunc(line, func(r rune) bool {
		for _, sep := range separators {
			if string(r) == sep {
				return true
			}
		}
		return false
	})
	return fields
}

func Map(lines []string, f Transform) []string {
	out := make([]string, 0, len(lines))
	for i, line := range lines {
		if line == "" {
			continue
		}
		out = append(out, f(line))
		_ = i
	}
	return out
}
