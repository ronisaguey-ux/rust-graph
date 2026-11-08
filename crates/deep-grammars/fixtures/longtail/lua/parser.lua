local token = require("scanner.token")

local Parser = {}
Parser.__index = Parser

local function describe(tok)
    if tok == nil then
        return "<eof>"
    elseif tok.kind == token.NUMBER then
        return string.format("number(%s)", tok.text)
    else
        return tok.text
    end
end

function Parser.parse(tokens)
    local self = setmetatable({ tokens = tokens, at = 1 }, Parser)
    local node = self:expression(1)
    if self:peek() ~= nil then
        error("trailing input at " .. describe(self:peek()))
    end
    return node
end

function Parser:peek()
    return self.tokens[self.at]
end

function Parser:expression(min)
    local left = self:primary()
    while true do
        local op = self:peek()
        if op == nil or op.precedence < min then
            break
        end
        self.at = self.at + 1
        left = { op = op.text, left = left, right = self:expression(op.precedence + 1) }
    end
    return left
end

function Parser:primary()
    local tok = self:peek()
    self.at = self.at + 1
    return { kind = tok.kind, value = tok.text }
end

return Parser
