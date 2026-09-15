DEM & Elevation Data Sources
To acquire elevation data (Digital Elevation Models) for these regions to convert into 3D heightmaps or TIN (Triangulated Irregular Network) meshes, you can use the following public datasets and tools:

* [USGS EarthExplorer](https://earthexplorer.usgs.gov/) – Primary source for 30-meter global SRTM (Shuttle Radar Topography Mission) and 10/30-meter NASADEM data covering all global regions listed above.

* [Copernicus Open Access Hub / DEM](https://dataspace.copernicus.eu/) – Offers Copernicus DEM (COP-DEM) at 30m (GLO-30) and 90m resolution globally, providing cleaner, high-detail terrain meshes over steep vertical cliffs and fjords.

* [OpenTopography](https://opentopography.org/) – Provides high-resolution LiDAR and DEM data for select regions (particularly US and Europe), including direct exports to GeoTIFF for heightmap creation.

* [TouchTerrain](https://touchterrain.geol.iastate.edu/) – Web-based interface that lets you select any bounding box on Earth and directly download 3D printable meshes or 16-bit DEM/TIN files for game engines.


While global open datasets generally max out at 30 meters, you can find 1-meter or 2-meter Digital Elevation Models (DEMs) through regional and national data initiatives, typically derived from high-resolution LiDAR or stereo imagery.

Here are the best sources for sub-30m elevation data:

* **USGS 3DEP (USA):** [The National Map Downloader](https://apps.nationalmap.gov/downloader/)
  The 3D Elevation Program provides seamless 1-meter bare-earth DEMs for most of the US. It is accessible through the **USGS National Map Downloader** or **OpenTopography**.
* **OpenTopography (Global/Curated):** [OpenTopography Data Portal](https://portal.opentopography.org/datasets)
  A major repository hosting high-resolution (often 1-meter or sub-meter) LiDAR datasets. It includes extensive US data, New Zealand's excellent LINZ LiDAR data, and select academic datasets from around the world.
* **ArcticDEM (North of 60°N):** [Polar Geospatial Center - ArcticDEM](https://www.pgc.umn.edu/data/arcticdem/)
  Offers continuous 2-meter resolution elevation models for all landmasses north of 60°N (ArcticDEM) and the entirety of Antarctica (REMA).
* **REMA (Antarctica):** [Polar Geospatial Center - REMA](https://www.pgc.umn.edu/data/rema/)
* **HRDEM (Canada):** [Canadian GIS Portal](https://app.geo.ca/en-ca/map-browser/record/957782bf-847c-4644-a757-e383c0057995) or via [OpenTopography](https://portal.opentopography.org/datasetMetadata?otCollectionID=OT.062025.3979.1)
  Natural Resources Canada provides 1-meter and 2-meter LiDAR-derived DEMs covering the southern, populated regions of the country.
* **European LiDAR Index:** [OpenDEM](https://opendem.info/)
* **European National Portals:** Several European countries (including the UK, Norway, Sweden, Finland, and Denmark) release open 1m-2m LiDAR data.
  You can find these datasets through their respective national mapping agencies or index sites like **OpenDEM**, which tracks high-resolution open datasets globally.
