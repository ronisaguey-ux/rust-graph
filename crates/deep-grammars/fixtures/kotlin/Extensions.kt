package com.example.util

import java.time.Duration

fun String.slug(): String =
    trim().lowercase().replace(Regex("[^a-z0-9]+"), "-").trim('-')

fun <T> List<T>.secondOrNull(): T? = if (size >= 2) this[1] else null

fun Duration.human(): String {
    val seconds = toSeconds()
    return when {
        seconds < 60 -> "${seconds}s"
        seconds < 3600 -> "${seconds / 60}m"
        else -> "${seconds / 3600}h"
    }
}

inline fun <T> List<T>.each(action: (T) -> Unit) {
    for (item in this) {
        action(item)
    }
}

fun List<Int>.sumEven(): Int {
    var total = 0
    for (value in this) {
        if (value % 2 != 0) continue
        total += value
    }
    return total
}
