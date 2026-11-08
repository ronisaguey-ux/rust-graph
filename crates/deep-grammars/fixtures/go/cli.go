package main

import (
	"fmt"
	"os"
	"strings"
)

var version = "0.1.0"

func main() {
	args := os.Args[1:]
	if len(args) == 0 {
		fmt.Println("usage: tool <command>")
		return
	}
	command := args[0]
	switch command {
	case "version", "-v":
		fmt.Println(version)
	case "echo":
		fmt.Println(strings.Join(args[1:], " "))
	default:
		fmt.Fprintf(os.Stderr, "unknown command: %s\n", command)
		os.Exit(1)
	}
}
