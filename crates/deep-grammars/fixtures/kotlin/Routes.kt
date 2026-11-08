package com.example.web

import com.example.models.User
import com.example.service.UserService

data class Response(val status: Int, val body: String)

class Router(private val users: UserService) {
    fun handle(method: String, path: String): Response {
        val parts = path.split("/").filter { it.isNotEmpty() }
        return when {
            method == "GET" && parts.isEmpty() -> Response(200, "index")
            method == "GET" && parts.size == 2 && parts[0] == "users" -> {
                val id = parts[1].toLongOrNull()
                if (id == null) {
                    Response(400, "bad id")
                } else {
                    val user = findUser(id)
                    if (user == null) Response(404, "missing") else Response(200, user.name)
                }
            }
            else -> Response(404, "not found")
        }
    }

    private fun findUser(id: Long): User? {
        return try {
            users.load(id)
        } catch (error: IllegalArgumentException) {
            null
        }
    }

    fun describe(response: Response): String {
        return when (response.status) {
            in 200..299 -> "ok"
            404 -> "missing"
            else -> "error ${response.status}"
        }
    }
}
