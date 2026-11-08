using System.Collections.Generic;
using System.Linq;
using Example.Models;
using Microsoft.AspNetCore.Mvc;

namespace Example.Web
{
    [ApiController]
    [Route("api/items")]
    public class ItemsController : ControllerBase
    {
        private readonly IRepository repository;

        public ItemsController(IRepository repository)
        {
            this.repository = repository;
        }

        [HttpGet]
        public ActionResult<IEnumerable<string>> List(string query, int limit = 20)
        {
            var names = repository.All()
                .Where(item => item.Active)
                .Select(item => item.Name)
                .Take(limit)
                .ToList();

            if (!string.IsNullOrWhiteSpace(query))
            {
                names = names.Where(name => name.Contains(query)).ToList();
            }

            return Ok(names);
        }

        [HttpPost]
        public IActionResult Create(Item item)
        {
            if (item == null)
            {
                return BadRequest("missing item");
            }
            repository.Save(item);
            return Created($"/api/items/{item.Id}", item);
        }
    }
}
