package com.example.model

import java.time.Instant

sealed interface Command {
    val issuedAt: Instant
}

data class Create(val name: String, override val issuedAt: Instant) : Command
data class Rename(val id: Long, val name: String, override val issuedAt: Instant) : Command
data class Delete(val id: Long, override val issuedAt: Instant) : Command

enum class Priority(val weight: Int) {
    LOW(1),
    NORMAL(5),
    HIGH(10);

    fun heavierThan(other: Priority): Boolean = weight > other.weight

    companion object {
        fun parse(text: String): Priority? = entries.firstOrNull { it.name.equals(text, ignoreCase = true) }
    }
}

data class Task(
    val id: Long,
    val title: String,
    val priority: Priority = Priority.NORMAL,
    val tags: Set<String> = emptySet()
) {
    fun withTag(tag: String): Task = copy(tags = tags + tag)

    val isUrgent: Boolean
        get() = priority == Priority.HIGH
}

fun Task.summary(): String = "#$id $title [$priority]"

fun List<Task>.urgent(): List<Task> = filter { it.isUrgent }

fun List<Task>.sortedByPriority(): List<Task> = sortedByDescending { it.priority.weight }

sealed class Result<out T> {
    data class Ok<T>(val value: T) : Result<T>()
    data class Err(val message: String) : Result<Nothing>()
}

fun <T> Result<T>.valueOr(default: T): T = when (this) {
    is Result.Ok -> value
    is Result.Err -> default
}
