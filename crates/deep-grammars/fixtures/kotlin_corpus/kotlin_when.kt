package com.example.state

sealed class Event {
    data class Login(val user: String, val at: Long) : Event()
    data class Logout(val user: String) : Event()
    data class Failure(val reason: Throwable) : Event()
    object Heartbeat : Event()
}

fun describe(event: Event): String = when (event) {
    is Event.Login -> "login ${event.user} at ${event.at}"
    is Event.Logout -> "logout ${event.user}"
    is Event.Failure -> "failure: ${event.reason.message}"
    Event.Heartbeat -> "alive"
}

fun describeAll(events: List<Event>): List<String> {
    return events.map { describe(it) }
}

fun bucket(code: Int): String = when (code) {
    in 100..199 -> "info"
    in 200..299 -> "success"
    in 300..399 -> "redirect"
    in 400..499 -> "client"
    in 500..599 -> "server"
    else -> "unknown"
}

fun coerce(value: Any): Int = when (value) {
    is Int -> value
    is String -> value.toIntOrNull() ?: 0
    is Number -> value.toInt()
    else -> throw IllegalArgumentException("not a number: $value")
}

fun label(event: Event): String {
    return when (event) {
        is Event.Login -> event.user
        is Event.Logout -> event.user
        is Event.Failure -> "error"
        Event.Heartbeat -> "beat"
    }
}
