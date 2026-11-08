package build

import groovy.json.JsonSlurper
import java.nio.file.Paths as Paths

class Builder {
    static final int MAX = 8
    String name = 'demo'
    def steps = []

    Builder(String name) {
        this.name = name
    }

    def add(String step) {
        steps << step
        return this
    }

    int count() {
        return steps.size()
    }
}

def builder = new Builder('release')
builder.add('compile').add('test')
def doubled = builder.steps.collect { it -> it + '!' }
if (doubled.size() > MAX) {
    println 'too many'
} else {
    println doubled
}
