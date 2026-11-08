-- Scanner plugin entry point.
local M = {}
local json = require("json")
local util = require("scanner.util")

local DEFAULTS = {
    depth = 12,
    follow_symlinks = false,
    exclude = { ".git", "node_modules", "target" },
    ["max-files"] = 50000,
}

function M.new(options)
    local self = setmetatable({}, { __index = M })
    self.options = options or DEFAULTS
    self.seen = {}
    return self
end

function M:scan(root)
    local count = 0
    for path in util.walk(root, self.options.depth) do
        if not self:skip(path) then
            count = count + 1
            self.seen[path] = true
        end
    end
    return count
end

function M:skip(path)
    for _, pattern in ipairs(self.options.exclude) do
        if string.find(path, pattern, 1, true) then
            return true
        end
    end
    return false
end

return M
