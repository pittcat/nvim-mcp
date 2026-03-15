-- Test configuration file
-- Contains custom tool definitions for testing

package.path = "./lua/?.lua;./lua/?/init.lua;" .. package.path

local M = require("nvim-mcp")
local MCP = M.MCP
M.setup({
    custom_tools = {
        format = {
            -- Do nothing actually. Test various of configuration options
            description = "Run format on the current buffer.",
            handler = function()
                return MCP.success("success")
            end,
        },
        save_buffer = {
            description = "Save a specific buffer by ID",
            parameters = {
                type = "object",
                properties = {
                    buffer_id = {
                        type = "integer",
                        description = "The buffer ID to save",
                        minimum = 1,
                    },
                },
                required = { "buffer_id" },
            },
            handler = function(params)
                local buf_id = params.buffer_id

                -- Validate buffer
                if not vim.api.nvim_buf_is_valid(buf_id) then
                    return MCP.error("INVALID_PARAMS", "Buffer " .. buf_id .. " is not valid")
                end

                local buf_name = vim.api.nvim_buf_get_name(buf_id)
                if buf_name == "" then
                    return MCP.error("INVALID_PARAMS", "Buffer " .. buf_id .. " has no associated file")
                end

                -- Save the buffer
                local success, err = pcall(function()
                    vim.api.nvim_buf_call(buf_id, function()
                        vim.cmd("write")
                    end)
                end)

                if success then
                    return MCP.success({
                        buffer_id = buf_id,
                        filename = buf_name,
                        message = "Buffer saved successfully",
                    })
                else
                    return MCP.error("INTERNAL_ERROR", "Failed to save buffer: " .. tostring(err))
                end
            end,
        },
    },
})
