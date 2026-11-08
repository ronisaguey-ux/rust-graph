package com.example.models

import java.time.Instant

enum class Status(val code: Int) {
    ACTIVE(1),
    SUSPENDED(2),
    CLOSED(3);

    fun isOpen(): Boolean = this == ACTIVE

    companion object {
        fun fromCode(code: Int): Status? = entries.firstOrNull { it.code == code }
    }
}

data class User(
    val id: Long,
    val name: String,
    val status: Status = Status.ACTIVE,
    val tags: List<String> = emptyList(),
    val createdAt: Instant = Instant.now()
) {
    fun rename(next: String): User {
        require(next.isNotBlank()) { "name must not be blank" }
        return copy(name = next)
    }

    val isActive: Boolean
        get() = status.isOpen()
}

sealed interface Event {
    val at: Instant

    data class Created(val id: Long, override val at: Instant) : Event
    data class Removed(val id: Long, override val at: Instant) : Event
}

typealias UserMap = Map<Long, User>
