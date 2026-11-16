Attributes {
  time {
    String _CoordinateAxisType "Time";
    Float64 actual_range 1.735758629924e+9, 1.794684142544e+9;
    String axis "T";
    String calendar "gregorian";
    String ioos_category "Time";
    String long_name "Centered Time";
    String standard_name "time";
    String time_origin "01-JAN-1970 00:00:00";
    String units "seconds since 1970-01-01T00:00:00Z";
  }
  altitude {
    String _CoordinateAxisType "Height";
    String _CoordinateZisPositive "up";
    Float64 actual_range 0.0, 0.0;
    String axis "Z";
    String ioos_category "Location";
    String long_name "Altitude";
    String positive "up";
    String standard_name "altitude";
    String units "m";
  }
  latitude {
    String _CoordinateAxisType "Lat";
    Float64 actual_range 0.003750000000003695, 59.99625;
    String axis "Y";
    String ioos_category "Location";
    String long_name "Latitude";
    String source_name "rows";
    String standard_name "latitude";
    String units "degrees_north";
  }
  longitude {
    String _CoordinateAxisType "Lon";
    Float64 actual_range -142.38375, -52.39125;
    String axis "X";
    String ioos_category "Location";
    String long_name "Longitude";
    String source_name "cols";
    String standard_name "longitude";
    String units "degrees_east";
  }
  kd_490 {
    Float64 _FillValue -32.768001556396484;
    Float64 colorBarMaximum 10.0;
    Float64 colorBarMinimum -10.0;
    Int32 cw_fraction_digits 14;
    String ioos_category "Optical Properties";
    String long_name "Diffuse Attenuation Coefficient at 490 nm";
    Float64 missing_value -32.768001556396484;
    String reference "Wang et al. (2009), https://doi.org/10.1029/2009JC005286";
    String units "m^-1";
  }
  l2_flags {
    Int32 _FillValue 2147483647;
    Float64 colorBarMaximum 150.0;
    Float64 colorBarMinimum 0.0;
    String ioos_category "Quality";
    String long_name "Level-2 Processing Flags";
  }
  swath_latitude {
    Float32 _FillValue -999.0;
    Float64 colorBarMaximum 90.0;
    Float64 colorBarMinimum -90.0;
    Int32 cw_fraction_digits 4;
    String ioos_category "Location";
    String long_name "Latitude";
    Float32 missing_value -999.0;
    String units "degrees_north";
  }
  swath_longitude {
    Float32 _FillValue -999.0;
    Float64 colorBarMaximum 180.0;
    Float64 colorBarMinimum -180.0;
    Int32 cw_fraction_digits 4;
    String ioos_category "Location";
    String long_name "Longitude";
    Float32 missing_value -999.0;
    String units "degrees_east";
  }
  qa_score {
    Float64 _FillValue -3.2767999172210693;
    Float64 colorBarMaximum 150.0;
    Float64 colorBarMinimum 0.0;
    Int32 cw_fraction_digits 16;
    String ioos_category "Quality";
    String long_name "Quality Assurance Score";
    Float64 missing_value -3.2767999172210693;
  }
  graphics {
    Byte _FillValue 0;
    String _Unsigned "true";
    Float64 colorBarMaximum 20.0;
    Float64 colorBarMinimum 0.0;
    String ioos_category "Optical Properties";
    String long_name "graphics overlay planes";
    Byte missing_value 0;
  }
  NC_GLOBAL {
    String cdm_data_type "Grid";
    String Conventions "CF-1.10, COARDS, ACDD-1.3";
    String creator_email "coastwatch.info@noaa.gov";
    String creator_name "NOAA CoastWatch";
    String creator_type "institution";
    String creator_url "https://coastwatch.noaa.gov";
    String cw_cwhdf_version "3.1";
    Float64 cw_et_affine 0.0, -0.0075, 0.0075, 0.0, -142.38375, 59.99625;
    Int32 cw_gctp_datum 12;
    Float64 cw_gctp_parm 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0;
    Int32 cw_gctp_sys 0;
    Int32 cw_gctp_zone 0;
    Int32 cw_pass_date 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771, 20771;
    String cw_pass_type "day";
    Float64 cw_polygon_latitude 59.99625, 59.99625, 59.99625, 59.99625, 59.99625, 44.998125, 30.000000000000004, 15.001875000000005, 0.003750000000003695, 0.003750000000003695, 0.003750000000003695, 0.003750000000003695, 0.003750000000003695, 15.001875000000005, 30.000000000000004, 44.998125, 59.99625;
    Float64 cw_polygon_longitude -142.38375, -119.88562499999999, -97.38749999999999, -74.889375, -52.39125, -52.39125, -52.39125, -52.39125, -52.39125, -74.889375, -97.38749999999999, -119.88562499999999, -142.38375, -142.38375, -142.38375, -142.38375, -142.38375;
    String cw_projection "Geographic";
    String cw_satellite "NOAA-21";
    String cw_sensor "VIIRS";
    Float64 cw_start_time 57196.524, 57280.641, 57282.431, 57364.758, 57366.548, 57450.666, 57452.455, 57536.573, 57538.362, 57622.48, 57624.27, 57706.597, 57708.387, 57792.504, 57794.294, 57878.412, 63000.624, 63084.742, 63086.532, 63168.859, 63170.649, 63254.766, 63256.556, 63340.673, 63342.463, 63424.791, 63426.581, 63510.698, 63512.488, 63596.605, 63598.395, 63680.723, 63682.512, 63766.63, 63768.419, 63852.537, 63854.327, 63936.654, 63938.444, 64022.561, 69144.774, 69228.891, 69230.681, 69314.799, 69316.588, 69400.706, 69402.495, 69484.823, 69486.613, 69570.73, 69572.52, 69656.637, 69658.427, 69740.755, 69742.544, 69826.662, 69828.452, 69912.569, 69914.359, 69996.686, 69998.476, 70082.593, 75204.806, 75288.923, 75290.713, 75374.83, 75376.62, 75458.948, 75460.737, 75544.855, 75546.645, 75630.762, 75632.552, 75716.669, 75718.459, 75800.786, 75802.576, 75886.694, 75888.483, 75972.601, 75974.39, 76056.718, 76058.508, 76142.625, 81264.837, 81348.955, 81350.745, 81434.862, 81436.652, 81518.979, 81520.769, 81604.886, 81606.676, 81690.794, 81692.583, 81774.911, 81776.701, 81860.818, 81862.608, 81946.725, 81948.515, 82030.843, 82032.632, 82116.75, 82118.539, 82202.657, 82204.447, 82288.564;
    String date_created "2026-11-14T18:18:34Z";
    Float64 Easternmost_Easting -52.39125;
    String geospatial_bounds "POLYGON((-142.38375 59.99625,-119.885625 59.99625,-97.3875 59.99625,-74.889375 59.99625,-52.39125 59.99625,-52.39125 44.998125,-52.39125 30.0,-52.39125 15.001875,-52.39125 0.00375,-74.889375 0.00375,-97.3875 0.00375,-119.885625 0.00375,-142.38375 0.00375,-142.38375 15.001875,-142.38375 30.0,-142.38375 44.998125,-142.38375 59.99625))";
    Float64 geospatial_lat_max 59.99625;
    Float64 geospatial_lat_min 0.003750000000003695;
    Float64 geospatial_lat_resolution 0.0075;
    String geospatial_lat_units "degrees_north";
    Float64 geospatial_lon_max -52.39125;
    Float64 geospatial_lon_min -142.38375;
    Float64 geospatial_lon_resolution 0.0075;
    String geospatial_lon_units "degrees_east";
    Float64 geospatial_vertical_max 0.0;
    Float64 geospatial_vertical_min 0.0;
    String geospatial_vertical_positive "up";
    String geospatial_vertical_units "m";
    Float64 grid_mapping_inverse_flattening 298.257223563;
    Float64 grid_mapping_longitude_of_prime_meridian 0.0;
    String grid_mapping_name "latitude_longitude";
    Float64 grid_mapping_semi_major_axis 6378137.0;
    String history "[cwutils 4.0.1.307 20241231_112006] cwcomposite -c kd_490/qa_score/l2_flags --method explicit --match ^(kd_490|longitude|latitude|qa_score|l2_flags) --inputs /data/data729/coastwatch/data/viirs/n21/nrt2/L3C/2026/318/tmppoaubkmn/inputlist /data/data729/coastwatch/data/viirs/n21/nrt2/L3C/2026/318/tmppoaubkmn/V2026318_D_N21_NRTNSW_US00_edgemask_kd490.hdf
[cwutils 4.0.1.307 20241231_112006] cwgraphics /data/data729/coastwatch/data/viirs/n21/nrt2/L3C/2026/318/tmppoaubkmn/V2026318_D_N21_NRTNSW_US00_edgemask_kd490.hdf
2026-11-16T23:43:07Z (local files)
2026-11-16T23:43:07Z https://coastwatch.noaa.gov/erddap/griddap/noaacwN21VIIRSkd490NSWUSDaily.das";
    String infoUrl "http://www.star.nesdis.noaa.gov";
    String institution "NOAA NESDIS STAR";
    String instrument "VIIRS";
    String keywords "altitude, attenuation, coefficient, cols, daily, data, day, diffuse, environmental, flags, graphics, imager, imager/radiometer, imaging, information, infrared, kd490, kd_490, l2_flags, latitude, level, level-2, longitude, merge, national, nesdis, noaa, noaa-21, optical, optical properties, overlay, planes, processing, properties, qa_score, quality, radiometer, rows, satellite, score, service, star, suite, swath_latitude, swath_longitude, time, viirs, visible";
    String license "The data may be used and redistributed for free but is not intended
for legal use, since it may contain inaccuracies. Neither the data
Contributor, ERD, NOAA, nor the United States Government, nor any
of their employees or contractors, makes any warranty, express or
implied, including warranties of merchantability and fitness for a
particular purpose, or assumes any legal liability for the accuracy,
completeness, or usefulness, of this information.";
    Float64 Northernmost_Northing 59.99625;
    String OBSERVED_PROPERTY "kd_490";
    String platform "NOAA-21";
    String source "NOAA-21_VIIRS_kd490";
    String sourceUrl "(local files)";
    Float64 Southernmost_Northing 0.003750000000003695;
    String standard_name_vocabulary "CF Standard Name Table v79";
    String summary "Daily Merge kd490 from NOAA-21 Visible and Infrared Imager/Radiometer Suite (VIIRS), MSL12 v1.8+";
    String testOutOfDate "now-15days";
    String time_coverage_end "2026-11-14T19:22:22Z";
    String time_coverage_start "2025-01-01T19:10:29Z";
    String title "Kd490 (NIR-SWIR), NOAA-21 VIIRS, NRT/Science Quality, 750m Sector US00 (United States), 2025-present, Daily";
    Float64 Westernmost_Easting -142.38375;
  }
}
