package com.example

import com.example.cache.Cache
import com.example.service.UserService
import kotlin.system.exitProcess

private const val VERSION = "0.1.0"

fun main(args: Array<String>) {
    if (args.isEmpty()) {
        println("usage: tool <command>")
        return
    }
    when (val command = args[0]) {
        "version", "-v" -> println(VERSION)
        "echo" -> println(args.drop(1).joinToString(" "))
        else -> {
            System.err.println("unknown command: $command")
            exitProcess(1)
        }
    }
}

fun parsePort(text: String, fallback: Int): Int {
    val port = text.toIntOrNull()
    return port ?: fallback
}

fun buildCache(): Cache<String, Int> {
    val cache = Cache<String, Int>()
    cache.put("one", 1)
    return cache
}
