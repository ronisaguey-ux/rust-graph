package worker

import (
	"context"
	"sync"
)

type Job struct {
	ID   int
	Body string
}

func Run(ctx context.Context, jobs <-chan Job, workers int) int {
	var wg sync.WaitGroup
	var mu sync.Mutex
	processed := 0
	for i := 0; i < workers; i++ {
		wg.Add(1)
		go func() {
			defer wg.Done()
			for {
				select {
				case job, ok := <-jobs:
					if !ok {
						return
					}
					mu.Lock()
					processed++
					mu.Unlock()
					_ = job
				case <-ctx.Done():
					return
				}
			}
		}()
	}
	wg.Wait()
	return processed
}
