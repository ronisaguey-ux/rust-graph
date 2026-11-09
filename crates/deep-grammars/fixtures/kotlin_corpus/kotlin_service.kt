package com.example.service

import com.example.model.Task
import com.example.store.TaskStore
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow

class TaskService(private val store: TaskStore) {

    suspend fun load(id: Long): Task {
        val cached = store.cache(id)
        if (cached != null) {
            return cached
        }
        val task = store.find(id) ?: throw NoSuchElementException("task $id")
        store.remember(task)
        return task
    }

    fun stream(): Flow<List<Task>> = flow {
        emit(store.all().sortedBy { it.id })
    }

    fun active(limit: Int = 50): List<Task> {
        val result = mutableListOf<Task>()
        for (task in store.all()) {
            if (!task.isUrgent) continue
            if (result.size >= limit) break
            result.add(task)
        }
        return result
    }

    fun rename(id: Long, name: String): Task {
        val task = store.find(id)
            ?: throw IllegalArgumentException("unknown task: $id")
        val updated = task.copy(title = name)
        store.remember(updated)
        return updated
    }

    fun report(limit: Int): String =
        active(limit)
            .groupBy { it.priority }
            .map { (priority, group) -> "$priority=${group.size}" }
            .joinToString(", ")
}
