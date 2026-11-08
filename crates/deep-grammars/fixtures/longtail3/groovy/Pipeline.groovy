package pipeline

import groovy.transform.CompileStatic

@CompileStatic
class Stage {
    String label
    boolean done = false

    def run(value) {
        try {
            switch (value) {
                case 0:
                    throw new IllegalStateException('zero')
                default:
                    done = true
            }
        } catch (IllegalStateException error) {
            println error.message
        } finally {
            println label
        }
        return done
    }
}
