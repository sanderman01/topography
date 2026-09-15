I'm making a game about flying a dirigible. I want to create a game map with interesting types of geography from the perspective of a player flying vfr. Bonus points if the terrain is hard to navigate when flying at low altitude. I want to use real earth elevation heightmaps as inspiration.

I want you to create a plan for collecting height data and merging and editing elevation data.

The rough process should be as follows:
* Select notable areas. Some examples are included in `docs/biomes.md` and `docs/locations.md`
* Write scripts or tools to retrieve data. Some example data sources are in `docs/elevation-data-sources.md`. Prefer high detail maps like 1m resoluton.
* Find more data sources from national agencies. For example, I tried retrieving an area of Karst topography from a dataset with 30m resolution, but 30m resolution is not detailed enough by far, so we need to
* Clean and curate the dataset. Discard elevation maps where resolution is low, or has high distortions or other artifacts. Human picks which areas are most interesting.

* Use a diffusion model to create the elevation map:
  * Establish Biome Control (ControlNet): To guarantee 9 distinct biomes appear exactly where I want them, use ControlNet. I will draw a low-resolution RGB segmentation map (e.g., red = jagged mountains, blue = river basin) and feed it into the ControlNet branch. The model will condition the generated terrain features strictly to my layout.
  * Curate the Dataset: Download real-world 1m resolution LiDAR DEMs that match target biomes. Slice these massive DEMs into 1024x1024 training tiles. Normalize the elevation data to 16-bit grayscale. Create paired color-coded segmentation maps for each tile so the model learns which features belong to which biome color.
  * Fine-Tune (LoRA):Train a Low-Rank Adaptation (LoRA) on tile dataset. This teaches the base diffusion model to output smooth, continuous 16-bit elevation gradients instead of high-contrast photographic imagery.
  * Tile Stitching (MultiDiffusion): stitch and blend areas together to form a single 32km x 32km elevation map composed of many sections/chunks.

* Use powers of two for distance or texture size where possible.
* All areas texture resolutions and area sizes should be configurable.
* Keep human in the loop wherever possible. I want to be able to do adjustments, partial re-runs, etc.
* Prefer bash or Rust for writing small custom cli tools.
* Elevation tiles may be subdivided further later to be rendered using a QuadTree in a Bevy game. (which is outside of the scope of this plan!)
