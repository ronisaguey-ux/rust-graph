package com.example.service

import com.example.models.User
import com.example.store.UserRepository

class UserService(
    private val repository: UserRepository
) {
    private var cache: MutableMap<Long, User> = mutableMapOf()

    suspend fun load(id: Long): User {
        cache[id]?.let { return it }
        val user = repository.find(id)
        if (user == null) {
            throw IllegalArgumentException("no user $id")
        }
        cache[id] = user
        return user
    }

    fun activeUsers(limit: Int = 50): List<User> {
        val result = mutableListOf<User>()
        for (user in repository.all()) {
            if (!user.isActive) {
                continue
            }
            if (result.size >= limit) {
                break
            }
            result.add(user)
        }
        return result
    }

    fun summarize(users: List<User>): String =
        users.groupBy { it.status }
            .map { (status, group) -> "$status=${group.size}" }
            .joinToString(", ")
}
