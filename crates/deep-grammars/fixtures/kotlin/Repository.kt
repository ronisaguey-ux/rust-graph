package com.example.store

import com.example.models.User

interface Repository<T, ID> {
    fun find(id: ID): T?
    fun save(item: T): T
    fun all(): List<T>
    fun delete(id: ID): Boolean
}

class MemoryRepository<T, ID>(
    private val keyOf: (T) -> ID
) : Repository<T, ID> {
    private val rows = LinkedHashMap<ID, T>()

    override fun find(id: ID): T? = rows[id]

    override fun save(item: T): T {
        rows[keyOf(item)] = item
        return item
    }

    override fun all(): List<T> = rows.values.toList()

    override fun delete(id: ID): Boolean = rows.remove(id) != null

    fun count(): Int = rows.size
}

class UserRepository : Repository<User, Long> {
    private val delegate = MemoryRepository<User, Long> { it.id }

    override fun find(id: Long): User? = delegate.find(id)
    override fun save(item: User): User = delegate.save(item)
    override fun all(): List<User> = delegate.all()
    override fun delete(id: Long): Boolean = delegate.delete(id)
}
