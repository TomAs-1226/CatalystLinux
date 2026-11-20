/*
 * A small, flat interface to the AprilTag library for Rust: a detector for
 * the 36h11 family (the one FRC fields use), detections as plain structs with
 * their pose already estimated, and a tag drawn as pixels. Rust never has to
 * mirror the library's own structs.
 */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include "apriltag.h"
#include "apriltag_pose.h"
#include "tag36h11.h"
#include "common/image_u8.h"
#include "common/matd.h"
#include "common/zarray.h"

typedef struct {
    int id;
    int hamming;
    float margin;
    double center[2];
    /* Corners in the image, counter-clockwise from the tag's bottom left. */
    double corners[4][2];
    /* Where the tag is from the camera (metres: x right, y down, z ahead),
     * the rotation from the camera's frame to the tag's, and the pose's
     * object-space error. has_pose is 0 when no size or lens was given. */
    double t[3];
    double r[9];
    double error;
    int has_pose;
} cat_tag;

typedef struct {
    apriltag_detector_t *td;
    apriltag_family_t *family;
} cat_detector;

void *cat_detector_new(int threads, float decimate)
{
    cat_detector *d = (cat_detector *)calloc(1, sizeof(cat_detector));
    if (!d)
        return NULL;
    d->family = tag36h11_create();
    d->td = apriltag_detector_create();
    /* Two bits corrected, as PhotonVision does by default for 36h11. */
    apriltag_detector_add_family_bits(d->td, d->family, 2);
    d->td->nthreads = threads > 0 ? threads : 1;
    d->td->quad_decimate = decimate > 0 ? decimate : 1.0f;
    d->td->quad_sigma = 0.0f;
    d->td->refine_edges = true;
    d->td->decode_sharpening = 0.25;
    return d;
}

void cat_detector_free(void *handle)
{
    cat_detector *d = (cat_detector *)handle;
    if (!d)
        return;
    apriltag_detector_destroy(d->td);
    tag36h11_destroy(d->family);
    free(d);
}

int cat_detect(void *handle, int width, int height, int stride, const uint8_t *pixels,
               double tagsize, double fx, double fy, double cx, double cy, cat_tag *out, int max)
{
    cat_detector *d = (cat_detector *)handle;
    if (!d || !pixels || !out || max <= 0)
        return 0;
    image_u8_t im = {.width = width, .height = height, .stride = stride, .buf = (uint8_t *)pixels};
    zarray_t *detections = apriltag_detector_detect(d->td, &im);
    int n = zarray_size(detections);
    int kept = 0;
    for (int i = 0; i < n && kept < max; i++) {
        apriltag_detection_t *det;
        zarray_get(detections, i, &det);
        cat_tag *tag = &out[kept++];
        memset(tag, 0, sizeof(*tag));
        tag->id = det->id;
        tag->hamming = det->hamming;
        tag->margin = det->decision_margin;
        tag->center[0] = det->c[0];
        tag->center[1] = det->c[1];
        memcpy(tag->corners, det->p, sizeof(tag->corners));
        if (tagsize > 0 && fx > 0 && fy > 0) {
            apriltag_detection_info_t info = {.det = det, .tagsize = tagsize, .fx = fx, .fy = fy, .cx = cx, .cy = cy};
            apriltag_pose_t pose;
            tag->error = estimate_tag_pose(&info, &pose);
            for (int k = 0; k < 3; k++)
                tag->t[k] = MATD_EL(pose.t, k, 0);
            for (int k = 0; k < 9; k++)
                tag->r[k] = MATD_EL(pose.R, k / 3, k % 3);
            matd_destroy(pose.t);
            matd_destroy(pose.R);
            tag->has_pose = 1;
        }
    }
    apriltag_detections_destroy(detections);
    return kept;
}

/* Tag `id` of the 36h11 family as pixels, one byte each (0 black, 255
 * white), `side` by `side` including its white border. Returns 0 if `out`
 * is too small or there is no such tag. */
int cat_tag_pixels(int id, uint8_t *out, int capacity, int *side)
{
    apriltag_family_t *family = tag36h11_create();
    int ok = 0;
    if (id >= 0 && (uint32_t)id < family->ncodes) {
        image_u8_t *im = apriltag_to_image(family, (uint32_t)id);
        if (im && im->width * im->height <= capacity) {
            for (int y = 0; y < im->height; y++)
                memcpy(out + y * im->width, im->buf + y * im->stride, (size_t)im->width);
            *side = im->width;
            ok = 1;
        }
        if (im)
            image_u8_destroy(im);
    }
    tag36h11_destroy(family);
    return ok;
}
