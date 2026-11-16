Attributes {
  time {
    String _CoordinateAxisType "Time";
    Float64 actual_range 1.1658384e+9, 1.794744e+9;
    String axis "T";
    String calendar "Gregorian";
    String ioos_category "Time";
    String long_name "Time";
    String standard_name "time";
    String time_origin "01-JAN-1970 00:00:00";
    String units "seconds since 1970-01-01T00:00:00Z";
  }
  latitude {
    String _CoordinateAxisType "Lat";
    Float64 actual_range 38.8749871947297, 50.6059751976539;
    String axis "Y";
    String ioos_category "Location";
    String long_name "Latitude";
    String standard_name "latitude";
    String units "degrees_north";
  }
  longitude {
    String _CoordinateAxisType "Lon";
    Float64 actual_range -92.4199507342304, -75.8816402880531;
    String axis "X";
    String ioos_category "Location";
    String long_name "Longitude";
    String standard_name "longitude";
    String units "degrees_east";
  }
  sst {
    Float32 _FillValue -99999.0;
    Float64 colorBarMaximum 32.0;
    Float64 colorBarMinimum 0.0;
    String ioos_category "Temperature";
    String long_name "Temperature";
    String standard_name "sea_water_temperature";
    String units "degree_C";
  }
  NC_GLOBAL {
    String cdm_data_type "Grid";
    String Conventions "CF-1.6, COARDS, ACDD-1.3";
    Float64 Easternmost_Easting -75.8816402880531;
    String GDAL "GDAL 3.4.3, released 2022/04/22";
    Float64 geospatial_lat_max 50.6059751976539;
    Float64 geospatial_lat_min 38.8749871947297;
    Float64 geospatial_lat_resolution 0.014015517327269056;
    String geospatial_lat_units "degrees_north";
    Float64 geospatial_lon_max -75.8816402880531;
    Float64 geospatial_lon_min -92.4199507342304;
    Float64 geospatial_lon_resolution 0.01401551732726889;
    String geospatial_lon_units "degrees_east";
    String grid_mapping_GeoTransform "-92.42695849289404 0.01401551732726498 0 50.61298295630733 0 -0.01401551732726498";
    Float64 grid_mapping_inverse_flattening 298.257223563;
    String grid_mapping_long_name "CRS definition";
    Float64 grid_mapping_longitude_of_prime_meridian 0.0;
    String grid_mapping_name "latitude_longitude";
    Float64 grid_mapping_semi_major_axis 6378137.0;
    String grid_mapping_spatial_ref "GEOGCS[\"WGS 84\",DATUM[\"WGS_1984\",SPHEROID[\"WGS 84\",6378137,298.257223563,AUTHORITY[\"EPSG\",\"7030\"]],AUTHORITY[\"EPSG\",\"6326\"]],PRIMEM[\"Greenwich\",0,AUTHORITY[\"EPSG\",\"8901\"]],UNIT[\"degree\",0.0174532925199433,AUTHORITY[\"EPSG\",\"9122\"]],AUTHORITY[\"EPSG\",\"4326\"]]";
    String history "Mon Nov 16 02:51:09 2026: ncrename -v Band1,sst /home/cw/opr/glsea3_b/mk_nc/output/2026_309_glsea_sst.nc
Mon Nov 16 02:51:08 2026: GDAL Create( /home/cw/opr/glsea3_b/mk_nc/output/2026_309_glsea_sst.nc, ... )
2026-11-16T23:49:43Z (local files)
2026-11-16T23:49:43Z https://apps.glerl.noaa.gov/erddap/griddap/GLSEA_ACSPO_GCS.das";
    String infoUrl "https://coastwatch.glerl.noaa.gov/glsea/glsea.html";
    String institution "CoastWatch Great Lakes Node";
    String keywords "ACSPO, analysis, data, earth, Earth Science > Oceans > Ocean Temperature > Water Temperature, environmental, glsea, great, great lakes, lakes, ocean, oceans, science, sea, sea_water_temperature, seawater, sst, surface, temperature, time, water";
    String keywords_vocabulary "GCMD Science Keywords";
    String license "The data may be used and redistributed for free but is not intended
for legal use, since it may contain inaccuracies. Neither the data
Contributor, ERD, NOAA, nor the United States Government, nor any
of their employees or contractors, makes any warranty, express or
implied, including warranties of merchantability and fitness for a
particular purpose, or assumes any legal liability for the accuracy,
completeness, or usefulness, of this information.";
    String NCO "netCDF Operators version 4.7.5 (Homepage = http://nco.sf.net, Code = https://github.com/nco/nco)";
    Float64 Northernmost_Northing 50.6059751976539;
    String source "2026_309_glsea.asc";
    String sourceUrl "(local files)";
    Float64 Southernmost_Northing 38.8749871947297;
    String standard_name_vocabulary "CF Standard Name Table v70";
    String summary "Sea Surface Temperature (SST) from Great Lakes Surface Environmental Analysis (GLSEA - ACSPO)";
    String testOutOfDate "now-3days";
    String time_coverage_end "2026-11-15T12:00:00Z";
    String time_coverage_start "2006-12-11T12:00:00Z";
    String title "Sea Surface Temperature (SST) from Great Lakes Surface Environmental Analysis (ACSPO GLSEA), The ACSPO GLSEA is the developed product that uses the Advanced Clear-sky Processor for Oceans (ACSPO) L3S-LEO SST as the input data source. The L3S-LEO SST is the level 3 Super Collated data from Low Earth Orbit satellites sensor including NPP, N20, and MetOp A/B/C. Geodetic coordinate system (LAT, LON), 2006-present";
    Float64 Westernmost_Easting -92.4199507342304;
  }
}
