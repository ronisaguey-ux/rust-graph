package reports

import groovy.json.JsonOutput

def names = ['alpha', 'beta', 'gamma']
def scores = [10, 20, 30]

def total = 0
for (def index = 0; index < names.size(); index++) {
    total = total + scores[index]
}

for (def name in names) {
    println name
}

while (total > 0) {
    total = total - 1
}

println JsonOutput.toJson(names)
