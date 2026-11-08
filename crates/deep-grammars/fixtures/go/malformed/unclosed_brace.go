package broken

type Config struct {
	Name string
	Retries int

func Load() Config {
	return Config{Name: "x", Retries: 3}
}
