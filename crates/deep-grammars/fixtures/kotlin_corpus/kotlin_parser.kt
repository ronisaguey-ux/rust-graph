package com.example.parse

sealed class Token {
    data class Word(val text: String) : Token()
    data class Number(val value: Int) : Token()
    object End : Token()
}

class Lexer(private val source: String) {
    private var position = 0

    fun tokenize(): List<Token> {
        val tokens = mutableListOf<Token>()
        while (position < source.length) {
            val char = source[position]
            when {
                char.isWhitespace() -> position++
                char.isDigit() -> tokens.add(readNumber())
                char.isLetter() -> tokens.add(readWord())
                else -> position++
            }
        }
        tokens.add(Token.End)
        return tokens
    }

    private fun readNumber(): Token.Number {
        val start = position
        while (position < source.length && source[position].isDigit()) {
            position++
        }
        return Token.Number(source.substring(start, position).toInt())
    }

    private fun readWord(): Token.Word {
        val start = position
        while (position < source.length && source[position].isLetter()) {
            position++
        }
        return Token.Word(source.substring(start, position))
    }
}

class Parser(private val tokens: List<Token>) {
    private var index = 0

    fun parse(): List<Int> {
        val numbers = mutableListOf<Int>()
        while (index < tokens.size) {
            when (val token = tokens[index++]) {
                is Token.Number -> numbers.add(token.value)
                is Token.Word -> Unit
                Token.End -> return numbers
            }
        }
        return numbers
    }
}
