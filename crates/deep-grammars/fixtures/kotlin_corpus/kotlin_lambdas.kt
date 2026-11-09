package com.example.build

import java.util.concurrent.locks.ReentrantLock

class Config {
    var host: String = "localhost"
    var port: Int = 8080
    val headers = mutableMapOf<String, String>()

    fun header(name: String, value: String) {
        headers[name] = value
    }
}

fun config(block: Config.() -> Unit): Config {
    val config = Config()
    config.block()
    return config
}

fun buildConnection(): String {
    val config = config {
        host = "api.example.com"
        port = 443
        header("accept", "application/json")
    }
    return "${config.host}:${config.port}"
}

fun <T, R> List<T>.foldEach(initial: R, combine: (R, T) -> R): R {
    var acc = initial
    for (item in this) {
        acc = combine(acc, item)
    }
    return acc
}

val lock = ReentrantLock()

fun <T> guarded(block: () -> T): T {
    lock.lock()
    try {
        return block()
    } finally {
        lock.unlock()
    }
}

fun total(values: List<Int>): Int = values.filter { it > 0 }.foldEach(0) { acc, value -> acc + value }

fun applyTwice(value: Int, op: (Int) -> Int): Int = op(op(value))

fun runAll(tasks: List<() -> Unit>): Int {
    var done = 0
    tasks.forEach { task ->
        guarded {
            task()
            done++
        }
    }
    return done
}
