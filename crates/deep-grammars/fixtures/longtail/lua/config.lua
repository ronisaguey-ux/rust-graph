-- Read and validate a scanner configuration file.
local json = require "json"

local schema = {
    name = "config",
    fields = {
        root = { type = "string", required = true },
        depth = { type = "number", default = 12 },
        languages = { type = "table", default = {} },
    },
}

local function validate(value, field, path)
    if field.required and value == nil then
        return nil, path .. " is required"
    end
    if value ~= nil and type(value) ~= field.type then
        return nil, string.format("%s must be a %s, got %s", path, field.type, type(value))
    end
    return value == nil and field.default or value
end

local function load(path)
    local file = assert(io.open(path, "r"))
    local text = file:read("*a")
    file:close()
    local data, err = json.decode(text)
    if not data then
        return nil, err
    end
    for name, field in pairs(schema.fields) do
        local ok, problem = validate(data[name], field, name)
        if not ok and problem then
            return nil, problem
        end
        data[name] = ok
    end
    return data
end

return { load = load, schema = schema }
